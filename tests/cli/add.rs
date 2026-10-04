//! `comline add`

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as Process;

use predicates::prelude::*;

use crate::util::*;

/// A minimal package at `dir`, declaring `Thing` in `src/models.ids`.
fn write_package(dir: &Path, name: &str) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("config.idp"),
        format!("congregation {name}\nspecification_version = 1\n"),
    )
    .unwrap();
    fs::write(
        dir.join("src/models.ids"),
        "struct Thing {\n    id: u64\n}\n",
    )
    .unwrap();
}

/// The fixture project, with a `shared` package next to it.
fn project_with_sibling(temp: &Path) -> PathBuf {
    let project = fixture_project(temp);
    write_package(&temp.join("shared"), "shared");
    project
}

fn uses_thing_from(project: &Path, dependency: &str) {
    fs::write(
        project.join("src/holder.ids"),
        format!("use {dependency}::models::Thing\n\nstruct Holder {{\n    thing: Thing\n}}\n"),
    )
    .unwrap();
}

fn check(project: &Path) -> assert_cmd::assert::Assert {
    comline_cmd().current_dir(project).arg("check").assert()
}

#[test]
fn adds_a_path_dependency_pinned_and_importable() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_sibling(temp.path());
    let before = fs::read_to_string(project.join("config.idp")).unwrap();

    comline_cmd()
        .current_dir(&project)
        .args(["add", "shared", "../shared"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Added shared to config.idp"))
        .stderr(predicate::str::contains("pinned: hash = \"blake3:"))
        .stderr(predicate::str::contains("trusting").not());

    let after = fs::read_to_string(project.join("config.idp")).unwrap();
    assert!(
        after.starts_with(before.trim_end()),
        "the rest is left as written:\n{after}"
    );
    assert!(after.contains("dependencies = {\n    shared = {\n        path = \"../shared\"\n        hash = \"blake3:"), "{after}");

    uses_thing_from(&project, "shared");
    check(&project).success();

    // The pin catches a change to the dependency.
    fs::write(
        temp.path().join("shared/src/models.ids"),
        "struct Thing {\n    id: u64\n    name: str\n}\n",
    )
    .unwrap();
    check(&project)
        .failure()
        .stderr(predicate::str::contains("hash mismatch"));
}

#[test]
fn no_hash_leaves_it_unpinned() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_sibling(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["add", "shared", "../shared", "--no-hash"])
        .assert()
        .success()
        .stderr(predicate::str::contains("not pinned"));

    assert!(!fs::read_to_string(project.join("config.idp"))
        .unwrap()
        .contains("hash ="));
    fs::write(
        temp.path().join("shared/src/models.ids"),
        "struct Thing {\n    id: u64\n    name: str\n}\n",
    )
    .unwrap();
    uses_thing_from(&project, "shared");
    check(&project).success();
}

#[test]
fn refuses_what_it_cant_add_and_leaves_config_idp_alone() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_sibling(temp.path());
    comline_cmd()
        .current_dir(&project)
        .args(["add", "shared", "../shared"])
        .assert()
        .success();
    let before = fs::read_to_string(project.join("config.idp")).unwrap();

    for (args, message) in [
        (
            vec!["add", "shared", "../shared"],
            "`shared` is already a dependency",
        ),
        (
            vec!["add", "elsewhere", "../nowhere"],
            "`../nowhere` isn't a Comline package",
        ),
        (
            vec!["add", "shared-types", "../shared"],
            "must be an identifier",
        ),
        // The fixture's own `src/other.ids`.
        (
            vec!["add", "other", "../shared"],
            "already has a `other` namespace",
        ),
    ] {
        comline_cmd()
            .current_dir(&project)
            .args(&args)
            .assert()
            .failure()
            .code(1)
            .stderr(predicate::str::contains(message));
        assert_eq!(
            fs::read_to_string(project.join("config.idp")).unwrap(),
            before,
            "{args:?}"
        );
    }
}

#[test]
fn a_package_that_doesnt_compile_isnt_added() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_sibling(temp.path());
    fs::write(temp.path().join("shared/src/models.ids"), "struct Broken {").unwrap();
    let before = fs::read_to_string(project.join("config.idp")).unwrap();

    comline_cmd()
        .current_dir(&project)
        .args(["add", "shared", "../shared"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("couldn't resolve the dependency"));
    assert_eq!(
        fs::read_to_string(project.join("config.idp")).unwrap(),
        before
    );
}

#[test]
fn git_needs_a_commit_and_a_version() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["add", "net", "--git", "https://example.test/net"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("--commit"));
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Process::new("git")
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
fn adds_a_git_dependency_at_a_pinned_commit() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    let repo = temp.path().join("net");
    write_package(&repo, "net");
    git(&repo, &["init", "--quiet"]);
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "--quiet", "-m", "net"]);
    let commit = git(&repo, &["rev-parse", "HEAD"]);
    let uri = format!("file://{}", repo.display());

    comline_cmd()
        .current_dir(&project)
        .args([
            "add",
            "net",
            "--git",
            &uri,
            "--commit",
            &commit,
            "--version",
            "1.0.0",
        ])
        .assert()
        .success()
        .stderr(predicate::str::contains("fetched into"));

    let config = fs::read_to_string(project.join("config.idp")).unwrap();
    assert!(
        config.contains(&format!(
            "    net = {{\n        version = \"1.0.0\"\n        uri = \"{uri}\"\n        commit = \"{commit}\"\n        hash = \"blake3:"
        )),
        "{config}"
    );
    assert_eq!(
        fs::read_dir(project.join(".comline/deps-cache"))
            .unwrap()
            .count(),
        1
    );

    uses_thing_from(&project, "net");
    check(&project).success();
}
