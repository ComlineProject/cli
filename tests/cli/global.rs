//! Global flags and cross-cutting output behaviour (`--plain`, `--quiet`,
//! `--path`) plus `comline completions`.

use predicates::prelude::*;

use crate::util::*;

#[test]
fn plain_output_has_no_color_or_symbols() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    let out = comline_cmd()
        .current_dir(&project)
        .args(["build", "--plain"])
        .assert()
        .success();
    let stderr = String::from_utf8_lossy(&out.get_output().stderr).into_owned();

    assert!(!stderr.contains('\u{1b}'), "no ANSI escapes: {stderr:?}");
    for glyph in ["•", "✓", "📦", "⬆", "🟢", "🔴"] {
        assert!(
            !stderr.contains(glyph),
            "no `{glyph}` in plain output: {stderr:?}"
        );
    }
    assert!(stderr.contains("Building project"));
    assert!(stderr.contains("Project built"));
}

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
