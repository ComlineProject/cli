use assert_cmd::Command;
use predicates::prelude::*;
use std::fs;
use std::path::{Path, PathBuf};

/// Build a `Command` for the `comline` binary under test.
///
/// Uses the `CARGO_BIN_EXE_comline` path Cargo exports for integration tests,
/// which works regardless of a custom `CARGO_TARGET_DIR` (unlike the deprecated
/// `Command::cargo_bin`).
fn comline_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_comline"))
}

/// Copy `tests/fixtures/simple_project` (two schema files) into a temp dir.
fn fixture_project(temp: &Path) -> PathBuf {
    let dest = temp.join("proj");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/simple_project");
    copy_dir(&src, &dest);
    dest
}

fn copy_dir(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for entry in fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).unwrap();
        }
    }
}

// ---------------------------------------------------------------------------
// new
// ---------------------------------------------------------------------------

#[test]
fn new_scaffolds_a_buildable_project() {
    let temp = tempfile::tempdir().unwrap();

    comline_cmd()
        .current_dir(&temp)
        .args(["new", "my_api"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Created"));

    let root = temp.path().join("my_api");
    assert!(root.join("config.idp").exists());
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
fn new_refuses_an_existing_directory() {
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
fn new_sanitizes_a_hyphenated_name_into_a_valid_package() {
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

// ---------------------------------------------------------------------------
// check
// ---------------------------------------------------------------------------

#[test]
fn check_passes_on_a_valid_project_without_writing_cas() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .arg("check")
        .assert()
        .success()
        .stderr(predicate::str::contains("Check passed"));

    assert!(
        !project.join(".comline").exists(),
        "check must not create the CAS store"
    );
}

#[test]
fn check_without_config_fails_with_precondition_code() {
    let temp = tempfile::tempdir().unwrap();

    comline_cmd()
        .current_dir(&temp)
        .arg("check")
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("is not a Comline project"));
}

#[test]
fn check_reports_a_broken_schema() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    fs::write(project.join("src/main.ids"), "struct Broken {").unwrap();

    comline_cmd()
        .current_dir(&project)
        .arg("check")
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("validation failed"));
}

// ---------------------------------------------------------------------------
// build
// ---------------------------------------------------------------------------

#[test]
fn build_freezes_an_initial_version() {
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

// ---------------------------------------------------------------------------
// generate
// ---------------------------------------------------------------------------

#[test]
fn generate_writes_one_file_per_schema() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Generated"));

    assert!(project.join("main.rust").exists());
    assert!(project.join("other.rust").exists());
}

#[test]
fn generate_rejects_an_unconfigured_target() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "python"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "no configured `code_generation` target",
        ));
}

// ---------------------------------------------------------------------------
// diff
// ---------------------------------------------------------------------------

#[test]
fn diff_reports_changes_between_two_versions() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();

    // add a struct, then rebuild to bump the version
    let schema = project.join("src/main.ids");
    let mut text = fs::read_to_string(&schema).unwrap();
    text.push_str("\nstruct Added {\n    x: string\n}\n");
    fs::write(&schema, text).unwrap();

    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();

    comline_cmd()
        .current_dir(&project)
        .args(["diff", "0.0.1", "HEAD"])
        .assert()
        .success()
        .stderr(predicate::str::contains("New features"))
        .stderr(predicate::str::contains("Added"));
}

#[test]
fn diff_without_a_build_fails_with_precondition_code() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["diff", "0.0.1", "HEAD"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("no builds found"));
}

#[test]
fn diff_rejects_an_unknown_ref() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();

    comline_cmd()
        .current_dir(&project)
        .args(["diff", "9.9.9"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("no built version matches"));
}

// ---------------------------------------------------------------------------
// clean
// ---------------------------------------------------------------------------

#[test]
fn clean_removes_the_cas_store() {
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
fn clean_dry_run_keeps_everything() {
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

// ---------------------------------------------------------------------------
// completions / global flags
// ---------------------------------------------------------------------------

#[test]
fn completions_are_written_to_stdout() {
    comline_cmd()
        .args(["completions", "bash"])
        .assert()
        .success()
        .stdout(predicate::str::contains("comline"))
        .stderr(predicate::str::is_empty());
}

#[test]
fn quiet_build_is_silent_on_success() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["--quiet", "build"])
        .assert()
        .success()
        .stdout(predicate::str::is_empty())
        .stderr(predicate::str::is_empty());
}

#[test]
fn path_flag_runs_outside_the_cwd() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&temp)
        .arg("--path")
        .arg(&project)
        .arg("check")
        .assert()
        .success()
        .stderr(predicate::str::contains("Check passed"));
}
