//! `comline generation clean` — removes generated code only, never `.comline/`.

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
        .args(["generation", "--target", "rust"])
        .assert()
        .success();
    assert!(project.join("generated/rust/api.rs").exists());
    assert!(project.join(".comline").is_dir());

    comline_cmd()
        .current_dir(&project)
        .args(["generation", "clean"])
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
        .args(["generation", "--target", "rust"])
        .assert()
        .success();

    comline_cmd()
        .current_dir(&project)
        .args(["generation", "clean", "--dry-run"])
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
        .args(["generation", "clean"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Nothing to clean"));

    assert!(project.join(".comline").is_dir());
}

#[test]
fn target_removes_only_that_languages_output() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    let config = project.join("config.idp");
    let idp = std::fs::read_to_string(&config).unwrap();
    std::fs::write(
        &config,
        idp.replace(
            "rust#1.85.0 = {}",
            "rust#1.85.0 = {}\n        typescript#5.0 = {}",
        ),
    )
    .unwrap();

    comline_cmd()
        .current_dir(&project)
        .arg("generation")
        .assert()
        .success();
    assert!(project.join("generated/rust/api.rs").exists());
    assert!(project.join("generated/typescript/api.ts").exists());

    comline_cmd()
        .current_dir(&project)
        .args(["generation", "clean", "--target", "rust"])
        .assert()
        .success();

    assert!(!project.join("generated/rust/api.rs").exists());
    assert!(
        project.join("generated/typescript/api.ts").exists(),
        "--target must leave the other languages' output alone"
    );
}

#[test]
fn rejects_an_unconfigured_target() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["generation", "clean", "--target", "python"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "no code-generation target matches `python`",
        ));
}
