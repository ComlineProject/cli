//! `comline deps list`

use std::fs;

use predicates::prelude::*;

use crate::util::*;

fn comline(project: &std::path::Path, args: &[&str]) -> assert_cmd::assert::Assert {
    comline_cmd().current_dir(project).args(args).assert()
}

#[test]
fn lists_a_path_dependency_pinned_and_unpinned() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_sibling(temp.path());
    write_package(&temp.path().join("scratch"), "scratch");
    comline(&project, &["deps", "add", "shared", "../shared"]).success();
    comline(
        &project,
        &["deps", "add", "scratch", "../scratch", "--no-hash"],
    )
    .success();
    let config = fs::read_to_string(project.join("config.idp")).unwrap();
    let hash = config
        .split_once("hash = \"blake3:")
        .unwrap()
        .1
        .split_once('"')
        .unwrap()
        .0;

    let assert = comline(&project, &["--plain", "deps", "list"]).success();
    let out = String::from_utf8(assert.get_output().stdout.clone()).unwrap();
    // One line each, by name, in columns, with no trailing space.
    assert_eq!(
        out,
        format!(
            "scratch  path ../scratch  unpinned\nshared   path ../shared   blake3:{}\n",
            &hash[..12]
        )
    );
}

#[test]
fn json_has_every_dependency_in_full() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_sibling(temp.path());
    write_package(&temp.path().join("scratch"), "scratch");
    comline(&project, &["deps", "add", "shared", "../shared"]).success();
    comline(
        &project,
        &["deps", "add", "scratch", "../scratch", "--no-hash"],
    )
    .success();
    let config = fs::read_to_string(project.join("config.idp")).unwrap();
    let hash = format!(
        "blake3:{}",
        config
            .split_once("hash = \"blake3:")
            .unwrap()
            .1
            .split_once('"')
            .unwrap()
            .0
    );

    let assert = comline(&project, &["deps", "list", "--json"]).success();
    let report: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(
        report,
        serde_json::json!({
            "dependencies": [
                {
                    "name": "scratch",
                    "source": { "kind": "path", "path": "../scratch" },
                    "hash": null,
                    "accept_unverified": {},
                },
                {
                    "name": "shared",
                    "source": { "kind": "path", "path": "../shared" },
                    "hash": hash,
                    "accept_unverified": {},
                },
            ]
        })
    );
}

#[test]
fn shows_what_is_accepted() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    let billing = temp.path().join("billing");
    fs::create_dir_all(billing.join("src")).unwrap();
    fs::write(
        billing.join("config.idp"),
        "congregation billing\nspecification_version = 1\n\
         settings = { annotations = { unverified = { allowed = true } } }\n",
    )
    .unwrap();
    fs::write(
        billing.join("src/api.ids"),
        "@handshake = \"none\"\n@unverified = \"the 2009 gateway\"\nprotocol Billing {\n    function ping();\n}\n",
    )
    .unwrap();
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

    comline(&project, &["--plain", "deps", "list"])
        .success()
        .stdout(predicate::str::contains(
            "accepts Billing (schema agreement and encoding agreement)",
        ));
    let assert = comline(&project, &["deps", "list", "--json"]).success();
    let report: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(
        report["dependencies"][0]["accept_unverified"],
        serde_json::json!({ "Billing": ["schema_agreement", "encoding_agreement"] })
    );
}

#[test]
fn only_reads_config_idp() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_sibling(temp.path());
    comline(&project, &["deps", "add", "shared", "../shared"]).success();
    // The dependency is gone, and its pin stale: listing says what the file says.
    fs::remove_dir_all(temp.path().join("shared")).unwrap();
    let _ = fs::remove_dir_all(project.join(".comline"));
    let before = fs::read_to_string(project.join("config.idp")).unwrap();

    comline(&project, &["--plain", "deps", "list"])
        .success()
        .stdout(predicate::str::starts_with(
            "shared  path ../shared  blake3:",
        ));
    assert_eq!(
        fs::read_to_string(project.join("config.idp")).unwrap(),
        before
    );
    assert!(!project.join(".comline").exists());
}

#[test]
fn no_dependencies_is_an_empty_list() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline(&project, &["--plain", "deps", "list"])
        .success()
        .stdout("")
        .stderr(predicate::str::contains("No dependencies declared"));
    let assert = comline(&project, &["deps", "list", "--json"]).success();
    let report: serde_json::Value = serde_json::from_slice(&assert.get_output().stdout).unwrap();
    assert_eq!(report, serde_json::json!({ "dependencies": [] }));
}
