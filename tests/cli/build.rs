//! `comline build`

use predicates::prelude::*;

use crate::util::*;

#[test]
fn freezes_an_initial_version() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["build", "--release"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Project built"))
        .stderr(predicate::str::contains("initial version 0.0.1"));

    assert!(project.join(".comline").is_dir());
}
