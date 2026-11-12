//! `comline deps pin`

use std::fs;
use std::path::Path;

use predicates::prelude::*;

use crate::util::*;

fn comline(project: &Path, args: &[&str]) -> assert_cmd::assert::Assert {
    comline_cmd().current_dir(project).args(args).assert()
}

/// What `shared` holds once its `Thing` has a name.
fn change_shared(temp: &Path) {
    fs::write(
        temp.join("shared/src/models.ids"),
        "struct Thing {\n    id: u64\n    name: string\n}\n",
    )
    .unwrap();
}

fn config(project: &Path) -> String {
    fs::read_to_string(project.join("config.idp")).unwrap()
}

/// The `blake3:...` a `hash = "..."` line of `config` holds.
fn pinned(config: &str) -> String {
    let rest = config.split_once("hash = \"").expect("a hash").1;
    rest.split_once('"').unwrap().0.to_string()
}

#[test]
fn a_changed_dependency_is_repinned_and_the_build_passes_again() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_sibling(temp.path());
    comline(&project, &["deps", "add", "shared", "../shared"]).success();
    uses_thing_from(&project, "shared");
    comline(&project, &["check"]).success();
    let before = config(&project);
    let old = pinned(&before);

    change_shared(temp.path());
    comline(&project, &["check"])
        .failure()
        .stderr(predicate::str::contains("hash mismatch"));

    let assert = comline(&project, &["deps", "pin", "shared"])
        .success()
        .stderr(predicate::str::contains("Pinned shared in config.idp"))
        .stderr(predicate::str::contains(format!("was: {old}")));
    let after = config(&project);
    let new = pinned(&after);
    assert_ne!(new, old);
    assert!(String::from_utf8_lossy(&assert.get_output().stderr).contains(&format!("now: {new}")));
    // Only the hash changed.
    assert_eq!(after, before.replace(&old, &new));
    comline(&project, &["check"]).success();
}

#[test]
fn it_is_a_no_op_when_the_content_is_unchanged() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_sibling(temp.path());
    comline(&project, &["deps", "add", "shared", "../shared"]).success();
    let before = config(&project);

    comline(&project, &["deps", "pin", "shared"])
        .success()
        .stderr(predicate::str::contains("already pinned to that content"))
        .stderr(predicate::str::contains(pinned(&before)))
        .stderr(predicate::str::contains("Pinned shared").not());
    assert_eq!(config(&project), before);
}

#[test]
fn an_entry_without_a_hash_gains_one() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_sibling(temp.path());
    comline(
        &project,
        &["deps", "add", "shared", "../shared", "--no-hash"],
    )
    .success();
    assert!(!config(&project).contains("hash ="));

    comline(&project, &["deps", "pin", "shared"])
        .success()
        .stderr(predicate::str::contains("was: not pinned"))
        .stderr(predicate::str::contains("now: blake3:"));
    let after = config(&project);
    assert!(
        after.contains("    shared = {\n        path = \"../shared\"\n        hash = \"blake3:"),
        "{after}"
    );

    // The pin catches the next change.
    change_shared(temp.path());
    uses_thing_from(&project, "shared");
    comline(&project, &["check"])
        .failure()
        .stderr(predicate::str::contains("hash mismatch"));
}

#[test]
fn a_dependency_that_doesnt_resolve_is_never_pinned() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_sibling(temp.path());
    comline(&project, &["deps", "add", "shared", "../shared"]).success();
    let before = config(&project);

    fs::write(temp.path().join("shared/src/models.ids"), "struct Broken {").unwrap();
    comline(&project, &["deps", "pin", "shared"])
        .failure()
        .code(1)
        .stderr(predicate::str::contains("couldn't resolve the dependency"));
    assert_eq!(config(&project), before);

    fs::remove_dir_all(temp.path().join("shared")).unwrap();
    comline(&project, &["deps", "pin", "shared"])
        .failure()
        .stderr(predicate::str::contains("couldn't resolve the dependency"));
    assert_eq!(config(&project), before);
}

#[test]
fn it_refuses_what_it_cant_pin_and_leaves_config_idp_alone() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_sibling(temp.path());
    comline(&project, &["deps", "add", "shared", "../shared"]).success();
    let before = config(&project);

    for (args, code, message) in [
        (
            vec!["deps", "pin", "nope"],
            1,
            "`nope` isn't a dependency in config.idp; it declares: shared",
        ),
        // A path dependency has no commit to move.
        (
            vec![
                "deps",
                "pin",
                "shared",
                "--commit",
                "abc",
                "--version",
                "1.0.0",
            ],
            1,
            "isn't a git dependency",
        ),
        // The two come together.
        (
            vec!["deps", "pin", "shared", "--commit", "abc"],
            2,
            "--version",
        ),
        (
            vec!["deps", "pin", "shared", "--version", "1.0.0"],
            2,
            "--commit",
        ),
    ] {
        comline(&project, &args)
            .failure()
            .code(code)
            .stderr(predicate::str::contains(message));
        assert_eq!(config(&project), before, "{args:?}");
    }
}

/// A package with `Billing` marked `@unverified`, as `accept.rs`'s.
fn write_unverified_package(dir: &Path, protocols: &[&str]) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("config.idp"),
        "congregation billing\nspecification_version = 1\n\
         settings = { annotations = { unverified = { allowed = true } } }\n",
    )
    .unwrap();
    let schema: String = protocols
        .iter()
        .map(|p| {
            format!(
                "@handshake = \"none\"\n@unverified = \"the 2009 gateway\"\n\
                 protocol {p} {{\n    function ping();\n}}\n\n"
            )
        })
        .collect();
    fs::write(dir.join("src/api.ids"), schema).unwrap();
}

#[test]
fn content_that_gives_up_more_needs_it_accepted() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    let billing = temp.path().join("billing");
    write_unverified_package(&billing, &["Billing"]);
    comline(
        &project,
        &[
            "deps",
            "add",
            "billing",
            "../billing",
            "--accept-unverified",
        ],
    )
    .success();

    write_unverified_package(&billing, &["Billing", "Ledger"]);
    let before = config(&project);

    // No terminal to ask: only what's new is shown, and nothing is written.
    comline(&project, &["deps", "pin", "billing"])
        .failure()
        .code(2)
        .stderr(predicate::str::contains("Ledger gives up"))
        .stderr(predicate::str::contains("Billing gives up").not())
        .stderr(predicate::str::contains("--accept-unverified"));
    assert_eq!(config(&project), before);

    comline(&project, &["deps", "pin", "billing", "--accept-unverified"])
        .success()
        .stderr(predicate::str::contains("Pinned billing in config.idp"))
        .stderr(predicate::str::contains("accept_unverified updated"));
    let after = config(&project);
    assert!(
        after.contains(
            "accept_unverified = { Billing = [\"schema_agreement\", \"encoding_agreement\"] \
             Ledger = [\"schema_agreement\", \"encoding_agreement\"] }"
        ),
        "{after}"
    );
    comline(&project, &["check"]).success();
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=test",
            "-c",
            "user.email=test@example.test",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

#[test]
fn a_git_dependency_moves_to_another_commit() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    let repo = temp.path().join("net");
    write_package(&repo, "net");
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "one"]);
    let first = git(&repo, &["rev-parse", "HEAD"]);
    fs::write(
        repo.join("src/models.ids"),
        "struct Thing {\n    id: u64\n    name: string\n}\n",
    )
    .unwrap();
    git(&repo, &["commit", "--quiet", "-am", "two"]);
    let second = git(&repo, &["rev-parse", "HEAD"]);
    let slashed = repo.display().to_string().replace('\\', "/");
    let uri = if slashed.starts_with('/') {
        format!("file://{slashed}")
    } else {
        format!("file:///{slashed}")
    };
    comline(
        &project,
        &[
            "deps",
            "add",
            "net",
            "--git",
            &uri,
            "--commit",
            &first,
            "--version",
            "1.0.0",
        ],
    )
    .success();
    let before = config(&project);
    let old = pinned(&before);

    comline(
        &project,
        &[
            "deps",
            "pin",
            "net",
            "--commit",
            &second,
            "--version",
            "1.1.0",
        ],
    )
    .success()
    .stderr(predicate::str::contains(format!("was: {old}")))
    .stderr(predicate::str::contains(format!(
        "moved to {second} (1.1.0)"
    )));
    let after = config(&project);
    assert!(
        after.contains(&format!(
            "    net = {{\n        version = \"1.1.0\"\n        uri = \"{uri}\"\n        commit = \"{second}\"\n        hash = \"blake3:"
        )),
        "{after}"
    );
    assert_ne!(pinned(&after), old);
    uses_thing_from(&project, "net");
    comline(&project, &["check"]).success();
}
