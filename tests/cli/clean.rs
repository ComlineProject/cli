//! `comline clean` — removes generated code only, never `.comline/`.

use predicates::prelude::*;

use crate::util::*;

#[test]
fn removes_generated_code_but_keeps_the_store() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();
    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success();
    assert!(project.join("generated/rust/api.rs").exists());
    assert!(project.join(".comline").is_dir());

    comline_cmd()
        .current_dir(&project)
        .arg("clean")
        .assert()
        .success()
        .stderr(predicate::str::contains("removed generated"));

    assert!(!project.join("generated").exists());
    assert!(
        project.join(".comline").is_dir(),
        "clean must not touch .comline/"
    );
}

#[test]
fn dry_run_lists_without_deleting() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success();

    comline_cmd()
        .current_dir(&project)
        .args(["clean", "--dry-run"])
        .assert()
        .success()
        .stderr(predicate::str::contains("would remove"));

    assert!(project.join("generated/rust/api.rs").exists());
}

#[test]
fn nothing_to_clean_when_no_generated_code() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();

    comline_cmd()
        .current_dir(&project)
        .arg("clean")
        .assert()
        .success()
        .stderr(predicate::str::contains("Nothing to clean"));

    assert!(project.join(".comline").is_dir());
}
