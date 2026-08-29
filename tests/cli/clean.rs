//! `comline clean`

use predicates::prelude::*;

use crate::util::*;

#[test]
fn removes_the_cas_store() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();
    assert!(project.join(".comline").exists());

    comline_cmd()
        .current_dir(&project)
        .arg("clean")
        .assert()
        .success()
        .stderr(predicate::str::contains("Clean complete"));

    assert!(!project.join(".comline").exists());
}

#[test]
fn dry_run_keeps_everything() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();

    comline_cmd()
        .current_dir(&project)
        .args(["clean", "--dry-run"])
        .assert()
        .success()
        .stderr(predicate::str::contains("would remove"));

    assert!(project.join(".comline").exists());
}
