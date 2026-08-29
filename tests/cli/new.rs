//! `comline new`

use std::fs;

use predicates::prelude::*;

use crate::util::*;

#[test]
fn scaffolds_a_buildable_project() {
    let temp = tempfile::tempdir().unwrap();

    comline_cmd()
        .current_dir(&temp)
        .args(["new", "my_api"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Created"));

    let root = temp.path().join("my_api");
    assert!(root.join("config.idp").exists());
    assert!(root.join("comline.toml").exists());
    assert!(root.join("src/main.ids").exists());
    assert!(root.join(".gitignore").exists());

    let config = fs::read_to_string(root.join("config.idp")).unwrap();
    assert!(config.contains("congregation my_api"));
    assert!(config.contains("code_generation"));

    // the scaffold must actually build
    comline_cmd()
        .current_dir(&root)
        .arg("build")
        .assert()
        .success();
}

#[test]
fn refuses_an_existing_directory() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("taken")).unwrap();

    comline_cmd()
        .current_dir(&temp)
        .args(["new", "taken"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("already exists"));
}

#[test]
fn sanitizes_a_hyphenated_name_into_a_valid_package() {
    let temp = tempfile::tempdir().unwrap();

    comline_cmd()
        .current_dir(&temp)
        .args(["new", "my-api"])
        .assert()
        .success();

    let root = temp.path().join("my-api");
    assert!(root.is_dir(), "directory keeps the original name");

    let config = fs::read_to_string(root.join("config.idp")).unwrap();
    assert!(config.contains("congregation my_api"));

    // the sanitized scaffold must still build
    comline_cmd()
        .current_dir(&root)
        .arg("build")
        .assert()
        .success();
}
