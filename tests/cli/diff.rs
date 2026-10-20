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
    let schema = project.join("src/api.ids");
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
        .args(["diff", "0.1.0-dev.1", "HEAD"])
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
        .args(["diff", "0.1.0-dev.1", "HEAD"])
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

#[test]
fn a_new_schema_file_sorting_first_is_an_addition_not_a_break() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();

    // `aaa` sorts before `api`: paired by position, `api`'s old schema would
    // be compared with this file and reported as removed.
    fs::write(
        project.join("src/aaa.ids"),
        "struct First {\n    x: string\n}\n",
    )
    .unwrap();

    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();

    comline_cmd()
        .current_dir(&project)
        .args(["diff", "0.1.0-dev.1", "HEAD"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Added struct `First`"))
        .stderr(predicate::str::contains("Breaking").not());
}
