//! `comline check`

use std::fs;

use predicates::prelude::*;

use crate::util::*;

#[test]
fn passes_on_a_valid_project_without_writing_cas() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .arg("check")
        .assert()
        .success()
        .stderr(predicate::str::contains("Check passed"));

    assert!(
        !project.join(".comline").exists(),
        "check must not create the CAS store"
    );
}

#[test]
fn without_config_fails_with_precondition_code() {
    let temp = tempfile::tempdir().unwrap();

    comline_cmd()
        .current_dir(&temp)
        .arg("check")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("is not a Comline project"));
}

#[test]
fn reports_a_broken_schema() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    fs::write(project.join("src/main.ids"), "struct Broken {").unwrap();

    comline_cmd()
        .current_dir(&project)
        .arg("check")
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("validation failed"));
}
