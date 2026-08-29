//! `comline generate`

use std::fs;

use predicates::prelude::*;

use crate::util::*;

#[test]
fn writes_one_file_per_schema_under_the_default_layout() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success()
        .stderr(predicate::str::contains("language: rust, version:"))
        .stderr(predicate::str::contains(
            "main.ids → generated/rust/main.rs",
        ))
        .stderr(predicate::str::contains("Generated"));

    // default layout: generated/{language}/{namespace}.{ext}
    assert!(project.join("generated/rust/main.rs").exists());
    assert!(project.join("generated/rust/other.rs").exists());
    // and it does NOT freeze a version
    assert!(!project.join(".comline").exists());
}

#[test]
fn honours_out_flag_and_a_flat_layout() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args([
            "generate",
            "--target",
            "rust",
            "--out",
            "bindings",
            "--layout",
            "{{namespace}}.{{ext}}",
        ])
        .assert()
        .success();

    assert!(project.join("bindings/main.rs").exists());
    assert!(project.join("bindings/other.rs").exists());
    assert!(!project.join("generated").exists());
}

#[test]
fn reads_out_from_comline_toml() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    fs::write(project.join("comline.toml"), "[generate]\nout = \"gen\"\n").unwrap();

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success();

    assert!(project.join("gen/rust/main.rs").exists());
}

#[test]
fn env_out_overrides_the_file() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    fs::write(project.join("comline.toml"), "[generate]\nout = \"gen\"\n").unwrap();

    comline_cmd()
        .current_dir(&project)
        .env("COMLINE_GENERATE_OUT", "envgen")
        .args(["generate", "--target", "rust"])
        .assert()
        .success();

    assert!(project.join("envgen/rust/main.rs").exists());
    assert!(!project.join("gen").exists());
}

#[test]
fn out_flag_beats_env_out() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .env("COMLINE_GENERATE_OUT", "envgen")
        .args(["generate", "--target", "rust", "--out", "flaggen"])
        .assert()
        .success();

    assert!(project.join("flaggen/rust/main.rs").exists());
    assert!(!project.join("envgen").exists());
}

#[test]
fn env_out_applies_in_a_multi_target_config() {
    // `--out` alone would error here (see out_flag_needs_target_when_several…);
    // the env var does not — it applies to every target.
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    fs::write(
        project.join("comline.toml"),
        "[[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.70.0\"\n\n\
         [[generate.target]]\nlanguage = \"python\"\nlang_version = \"3.11.0\"\n",
    )
    .unwrap();

    comline_cmd()
        .current_dir(&project)
        .env("COMLINE_GENERATE_OUT", "envgen")
        .args(["generate", "--target", "rust"])
        .assert()
        .success();

    assert!(project.join("envgen/rust/main.rs").exists());
}

#[test]
fn plain_uses_ascii_arrow() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust", "--plain"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "main.ids -> generated/rust/main.rs",
        ))
        .stderr(predicate::str::contains('⚙').not());
}

#[test]
fn out_flag_needs_target_when_several_are_configured() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    fs::write(
        project.join("comline.toml"),
        "[[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.70.0\"\n\n\
         [[generate.target]]\nlanguage = \"python\"\nlang_version = \"3.11.0\"\n",
    )
    .unwrap();

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--out", "x"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("need `--target"));
}

#[test]
fn rejects_an_unconfigured_target() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "python"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "no code-generation target matches `python`",
        ));
}
