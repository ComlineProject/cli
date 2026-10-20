//! `comline build`

use predicates::prelude::*;

use crate::util::*;

#[test]
fn freezes_an_initial_version() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success()
        .stderr(predicate::str::contains("Project built"))
        .stderr(predicate::str::contains("initial build 0.1.0-dev.1"))
        .stderr(predicate::str::contains(
            "next release: 0.1.0 (the first release)",
        ));

    assert!(project.join(".comline").is_dir());
}
