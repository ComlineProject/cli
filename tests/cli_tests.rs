use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;

#[test]
fn test_new_project_creation() {
    let temp_dir = tempfile::tempdir().unwrap();
    let project_name = "my_test_project";

    let mut cmd = Command::cargo_bin("comline").unwrap();
    cmd.current_dir(&temp_dir)
        .arg("new")
        .arg(project_name)
        .assert()
        .success();

    let project_path = temp_dir.path().join(project_name);
    assert!(project_path.exists());
    assert!(project_path.join("config.idp").exists());

    let config_content = fs::read_to_string(project_path.join("config.idp")).unwrap();
    assert!(config_content.contains(&format!("congregation {}", project_name)));
    assert!(config_content.contains("specification_version = 1"));
}

#[test]
fn test_check_valid_project() {
    let temp_dir = tempfile::tempdir().unwrap();
    let project_name = "valid_project";

    // Create a valid project first
    let mut cmd = Command::cargo_bin("comline").unwrap();
    cmd.current_dir(&temp_dir)
        .arg("new")
        .arg(project_name)
        .assert()
        .success();

    let project_path = temp_dir.path().join(project_name);

    // Run check
    let mut cmd_check = Command::cargo_bin("comline").unwrap();
    cmd_check
        .current_dir(&project_path)
        .arg("check")
        .assert()
        .success()
        .stdout(predicate::str::contains("Check passed!"));
}

#[test]
fn test_build_project() {
    let temp_dir = tempfile::tempdir().unwrap();
    let project_name = "buildable_project";

    // Create a valid project first
    let mut cmd = Command::cargo_bin("comline").unwrap();
    cmd.current_dir(&temp_dir)
        .arg("new")
        .arg(project_name)
        .assert()
        .success();

    let project_path = temp_dir.path().join(project_name);

    // Run build
    let mut cmd_build = Command::cargo_bin("comline").unwrap();
    cmd_build
        .current_dir(&project_path)
        .arg("build")
        .arg("--release")
        .assert()
        .success()
        .stdout(predicate::str::contains("Project built successfully!"));
}

#[test]
fn test_check_fail_no_config() {
    let temp_dir = tempfile::tempdir().unwrap();

    let mut cmd = Command::cargo_bin("comline").unwrap();
    cmd.current_dir(&temp_dir)
        .arg("check")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "Comline project (missing config.idp)",
        ));
}
