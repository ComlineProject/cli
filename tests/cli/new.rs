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
    // An editor validates comline.toml against its schema from the first line.
    let comline_toml = fs::read_to_string(root.join("comline.toml")).unwrap();
    assert_eq!(
        comline_toml.lines().next(),
        Some("#:schema https://comlineproject.github.io/docs/schemas/comline-toml/0.x.json"),
        "{comline_toml}"
    );
    assert!(root.join("src/greeting.ids").exists());
    // The version history is committed; only fetched dependencies are ignored.
    let gitignore = fs::read_to_string(root.join(".gitignore")).unwrap();
    assert!(
        gitignore.lines().any(|l| l == ".comline/deps-cache/"),
        "{gitignore}"
    );
    assert!(!gitignore.lines().any(|l| l == ".comline/"), "{gitignore}");
    assert!(
        gitignore.lines().any(|l| l == "docs/api/html/"),
        "{gitignore}"
    );

    let config = fs::read_to_string(root.join("config.idp")).unwrap();
    assert!(config.contains("congregation my_api"));
    // The package's identity, random, on the line after its name.
    let id = config.lines().nth(1).unwrap();
    assert!(
        id.starts_with("id = \"") && id.len() == "id = \"\"".len() + 32,
        "{config}"
    );
    assert!(config.contains("code_generation"));
    // The widest promise: the Rust generator's floor.
    assert!(config.contains("rust#1.85.0 = {}"), "{config}");

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
