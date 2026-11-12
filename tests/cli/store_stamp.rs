//! Every command that reads the `.comline/` store checks its format stamp
//! first, and refuses a store in another format instead of decoding it.

use std::fs;
use std::path::Path;

use predicates::prelude::*;

use crate::util::*;

/// The commands that read the store, as arguments. `doc open` renders
/// through `doc` and opens existing output without reading, so `doc` and
/// `doc --version` stand for it.
const READERS: &[&[&str]] = &[
    &["diff", "0.1.0-dev.1", "HEAD"],
    &["doc"],
    &["doc", "--version", "HEAD"],
    &["generation"],
    &["generation", "--list"],
    &["inspect"],
    &["inspect", "--json"],
    &["release", "--yes"],
];

fn built_project(temp: &Path) -> std::path::PathBuf {
    let project = fixture_project(temp);
    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();
    project
}

fn stamp(project: &Path) -> std::path::PathBuf {
    project.join(".comline/FORMAT_VERSION")
}

#[test]
fn a_wrong_stamp_is_refused_by_every_read_command() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    fs::write(stamp(&project), "1").unwrap();

    for args in READERS {
        comline_cmd()
            .current_dir(&project)
            .args(*args)
            .assert()
            .failure()
            .code(2)
            .stderr(predicate::str::contains(
                "the version history in `.comline/` is in IR format `1`, but this comline \
                 reads and writes format",
            ))
            .stderr(predicate::str::contains("run `comline build`"));
    }
    // Refusing changed nothing.
    assert_eq!(fs::read_to_string(stamp(&project)).unwrap(), "1");
}

#[test]
fn a_missing_stamp_with_history_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    fs::remove_file(stamp(&project)).unwrap();

    for args in READERS {
        comline_cmd()
            .current_dir(&project)
            .args(*args)
            .assert()
            .failure()
            .code(2)
            .stderr(predicate::str::contains("is in IR format unstamped"));
    }
}

#[test]
fn build_migrates_a_wrong_stamp_and_reads_work_again() {
    let temp = tempfile::tempdir().unwrap();
    let project = built_project(temp.path());
    fs::write(stamp(&project), "1").unwrap();

    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();
    comline_cmd()
        .current_dir(&project)
        .args(["diff", "0.1.0-dev.1", "HEAD"])
        .assert()
        .success();
}

#[test]
fn no_stamp_and_no_history_is_nothing_built_not_a_format_error() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    fs::create_dir_all(project.join(".comline/objects")).unwrap();

    comline_cmd()
        .current_dir(&project)
        .args(["diff", "0.1.0-dev.1", "HEAD"])
        .assert()
        .failure()
        .code(2)
        .stderr(predicate::str::contains("no builds found"))
        .stderr(predicate::str::contains("IR format").not());
}
