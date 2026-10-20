//! `comline accept`, and `comline add` on a dependency with `@unverified`
//! protocols.

use std::fs;
use std::path::Path;

use predicates::prelude::*;

use crate::util::*;

const PRECONDITION: i32 = 2;

/// A package at `dir` whose listed protocols run no handshake, each marked.
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

fn comline(project: &Path, args: &[&str]) -> assert_cmd::assert::Assert {
    comline_cmd().current_dir(project).args(args).assert()
}

#[test]
fn add_shows_what_is_given_up_and_needs_it_accepted() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    write_unverified_package(&temp.path().join("billing"), &["Billing"]);
    let before = fs::read_to_string(project.join("config.idp")).unwrap();

    // No terminal to ask: refused, with what it gives up and the way through.
    comline(&project, &["add", "billing", "../billing"])
        .failure()
        .code(PRECONDITION)
        .stderr(predicate::str::contains(
            "Billing gives up schema agreement and encoding agreement: \"the 2009 gateway\"",
        ))
        .stderr(predicate::str::contains("--accept-unverified"));
    assert_eq!(
        fs::read_to_string(project.join("config.idp")).unwrap(),
        before
    );

    comline(
        &project,
        &["add", "billing", "../billing", "--accept-unverified"],
    )
    .success()
    .stderr(predicate::str::contains("Added billing to config.idp"));
    let after = fs::read_to_string(project.join("config.idp")).unwrap();
    assert!(
        after.contains(
            "        accept_unverified = { Billing = [\"schema_agreement\", \"encoding_agreement\"] }\n"
        ),
        "{after}"
    );
    comline(&project, &["check"]).success();
}

#[test]
fn accept_takes_in_what_an_update_gives_up() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    let billing = temp.path().join("billing");
    write_unverified_package(&billing, &["Billing"]);
    comline(
        &project,
        &[
            "add",
            "billing",
            "../billing",
            "--no-hash",
            "--accept-unverified",
        ],
    )
    .success();

    // The dependency now gives up more: the build stops until it's accepted.
    write_unverified_package(&billing, &["Billing", "Ledger"]);
    comline(&project, &["check"])
        .failure()
        .stderr(predicate::str::contains("protocol `Ledger` is unverified"));

    comline(&project, &["accept", "billing"])
        .failure()
        .code(PRECONDITION)
        .stderr(predicate::str::contains("Ledger gives up"))
        .stderr(predicate::str::contains("Billing gives up").not())
        .stderr(predicate::str::contains("--yes"));

    comline(&project, &["accept", "billing", "--yes"])
        .success()
        .stderr(predicate::str::contains("Accepted Ledger from billing"));
    let manifest = fs::read_to_string(project.join("config.idp")).unwrap();
    assert!(
        manifest.contains(
            "accept_unverified = { Billing = [\"schema_agreement\", \"encoding_agreement\"] \
             Ledger = [\"schema_agreement\", \"encoding_agreement\"] }"
        ),
        "{manifest}"
    );
    comline(&project, &["check"]).success();

    comline(&project, &["accept", "billing::Ledger", "--yes"])
        .success()
        .stderr(predicate::str::contains("Nothing to accept"));
}

#[test]
fn accept_refuses_what_it_cant_find() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    write_unverified_package(&temp.path().join("billing"), &["Billing"]);
    comline(
        &project,
        &["add", "billing", "../billing", "--accept-unverified"],
    )
    .success();

    comline(&project, &["accept", "nope", "--yes"])
        .failure()
        .stderr(predicate::str::contains("`nope` isn't a dependency"));
    comline(&project, &["accept", "billing::Nope", "--yes"])
        .failure()
        .stderr(predicate::str::contains("no unverified protocol `Nope`"));
}
