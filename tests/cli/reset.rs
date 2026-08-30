//! `comline reset` — discards the version history (`.comline/`) behind a guard.

use predicates::prelude::*;

use crate::util::*;

#[test]
fn force_removes_the_store_and_generated_code() {
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
    assert!(project.join(".comline").is_dir());
    assert!(project.join("generated/rust/main.rs").exists());

    comline_cmd()
        .current_dir(&project)
        .args(["reset", "--force"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Reset complete"));

    assert!(!project.join(".comline").exists());
    assert!(!project.join("generated").exists());
}

#[test]
fn without_force_and_no_tty_it_refuses() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();

    comline_cmd()
        .current_dir(&project)
        .arg("reset")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("without confirmation"));

    assert!(
        project.join(".comline").is_dir(),
        "a refused reset must leave .comline/ intact"
    );
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
        .args(["reset", "--dry-run"])
        .assert()
        .success()
        .stderr(predicate::str::contains("would remove .comline"))
        .stderr(predicate::str::contains("currently 0.0.1"));

    assert!(project.join(".comline").is_dir());
}
