//! `comline diff`

use std::fs;

use predicates::prelude::*;

use crate::util::*;

#[test]
fn reports_changes_between_two_versions() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();

    // add a struct, then rebuild to bump the version
    let schema = project.join("src/main.ids");
    let mut text = fs::read_to_string(&schema).unwrap();
    text.push_str("\nstruct Added {\n    x: string\n}\n");
    fs::write(&schema, text).unwrap();

    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();

    comline_cmd()
        .current_dir(&project)
        .args(["diff", "0.0.1", "HEAD"])
        .assert()
        .success()
        .stderr(predicate::str::contains("New features"))
        .stderr(predicate::str::contains("Added"));
}

#[test]
fn without_a_build_fails_with_precondition_code() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["diff", "0.0.1", "HEAD"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("no builds found"));
}

#[test]
fn rejects_an_unknown_ref() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();

    comline_cmd()
        .current_dir(&project)
        .args(["diff", "9.9.9"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("no built version matches"));
}
