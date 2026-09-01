//! Shared helpers for the `comline` integration tests.

use std::fs;
use std::path::{Path, PathBuf};

use assert_cmd::Command;

/// A `Command` for the `comline` binary under test.
///
/// Uses the `CARGO_BIN_EXE_comline` path Cargo exports for integration tests,
/// which works regardless of a custom `CARGO_TARGET_DIR` (unlike the deprecated
/// `Command::cargo_bin`).
pub fn comline_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_comline"))
}

/// Copy `tests/fixtures/simple_project` into a fresh temp dir.
///
/// Only the project inputs are copied — `config.idp` and `src/`. Anything a
/// previous `comline` run may have left in the fixture (`.comline/`, generated
/// files) is skipped, so the tests stay hermetic even if someone has run the
/// CLI against the fixture locally.
pub fn fixture_project(temp: &Path) -> PathBuf {
    copy_fixture("simple_project", temp)
}

/// Like [`fixture_project`] for an arbitrary fixture under `tests/fixtures/`.
pub fn copy_fixture(name: &str, temp: &Path) -> PathBuf {
    let dest = temp.join("proj");
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    fs::create_dir_all(&dest).unwrap();
    fs::copy(src.join("config.idp"), dest.join("config.idp")).unwrap();
    copy_dir(&src.join("src"), &dest.join("src"));
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
