//! `comline release`

use std::fs;
use std::path::Path;

use predicates::prelude::*;

use crate::util::*;

const PRECONDITION: i32 = 2;

fn comline(project: &Path, args: &[&str]) -> assert_cmd::assert::Assert {
    comline_cmd().current_dir(project).args(args).assert()
}

fn add_struct(project: &Path, name: &str) {
    let schema = project.join("src/api.ids");
    let mut text = fs::read_to_string(&schema).unwrap();
    text.push_str(&format!("\nstruct {name} {{\n    x: string\n}}\n"));
    fs::write(&schema, text).unwrap();
}

#[test]
fn releases_what_was_built_and_then_has_nothing_to_release() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    // It builds first: no `comline build` needed.
    comline(&project, &["release"])
        .success()
        .stderr(predicate::str::contains("Released 0.1.0"))
        .stderr(predicate::str::contains("the first release"));
    comline(&project, &["release"])
        .failure()
        .stderr(predicate::str::contains("0.1.0 is already released"));

    add_struct(&project, "Added");
    comline(&project, &["build"])
        .success()
        .stderr(
            predicate::str::contains("0.1.0 → 0.1.1-dev.1")
                .or(predicate::str::contains("0.1.0 -> 0.1.1-dev.1")),
        )
        .stderr(predicate::str::contains(
            "next release: 0.1.1 (new features since 0.1.0)",
        ));
    comline(&project, &["release"])
        .success()
        .stderr(predicate::str::contains("Released 0.1.1"))
        .stderr(predicate::str::contains("0.1.0 -> 0.1.1"))
        .stderr(predicate::str::contains("Added struct `Added`"))
        .stderr(predicate::str::contains("minor bump"));
}

#[test]
fn raising_needs_a_reason_and_a_major_raise_is_confirmed() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    comline(&project, &["release"]).success();
    add_struct(&project, "Added");

    comline(&project, &["release", "--raise", "major"])
        .failure()
        .stderr(predicate::str::contains("--raise needs --reason"));
    comline(
        &project,
        &["release", "--raise", "major", "--reason", "retries changed"],
    )
    .failure()
    .code(PRECONDITION)
    .stderr(predicate::str::contains("pass `--yes`"));
    comline(
        &project,
        &[
            "release",
            "--raise",
            "major",
            "--reason",
            "retries changed",
            "--yes",
        ],
    )
    .success()
    .stderr(predicate::str::contains("Released 0.2.0"))
    .stderr(predicate::str::contains("major bump, raised from minor"))
    .stderr(predicate::str::contains("reason: retries changed"));
}

#[test]
fn release_1_0_says_what_it_commits_to() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    comline(&project, &["release"]).success();

    comline(&project, &["release", "2.0"])
        .failure()
        .stderr(predicate::str::contains("the only one you name is `1.0`"));
    comline(&project, &["release", "1.0"])
        .failure()
        .code(PRECONDITION)
        .stderr(predicate::str::contains(
            "a breaking change is a major release",
        ))
        .stderr(predicate::str::contains("pass `--yes`"));
    comline(&project, &["release", "1.0", "--yes"])
        .success()
        .stderr(predicate::str::contains("Released 1.0.0"))
        .stderr(predicate::str::contains("stable"));
    comline(&project, &["release", "1.0", "--yes"])
        .failure()
        .stderr(predicate::str::contains("already stable"));
}
