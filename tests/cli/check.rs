//! `comline check`

use std::fs;

use predicates::prelude::*;

use crate::util::*;

#[test]
fn passes_on_a_valid_project_without_writing_cas() {
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
fn without_config_fails_with_precondition_code() {
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
fn reports_a_broken_schema() {
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

#[test]
fn a_broken_schema_reports_a_source_snippet_not_a_debug_dump() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    fs::write(project.join("src/main.ids"), "struct Greeting {\n    / name: string\n}\n").unwrap();

    comline_cmd()
        .current_dir(&project)
        .arg("check")
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("ParseError {").not())
        .stderr(predicate::str::contains("FailedNode(").not())
        .stderr(predicate::str::contains("main.ids"))
        .stderr(predicate::str::contains("unexpected token"))
        .stderr(predicate::str::is_match(r"\d+:\d+").unwrap());
}

#[test]
fn a_broken_config_idp_reports_a_source_snippet_not_a_debug_dump() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    fs::write(project.join("config.idp"), "congregation test\nspecification_version =\n").unwrap();

    comline_cmd()
        .current_dir(&project)
        .arg("check")
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("ParseError {").not())
        .stderr(predicate::str::contains("FailedNode(").not())
        .stderr(predicate::str::contains("config.idp"));
}

#[test]
fn std_is_usable_and_checked() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    fs::write(
        project.join("src/exchange.ids"),
        "use std::http::{Request, Response}\nuse std::validators::StringBounds\n\nstruct Exchange {\n    request: Request\n    response: Response\n    @validators = [StringBounds(min_chars = 1)]\n    label: str\n}\n",
    )
    .unwrap();

    comline_cmd()
        .current_dir(&project)
        .arg("check")
        .assert()
        .success()
        .stderr(predicate::str::contains("Check passed"))
        .stderr(predicate::str::contains("WARN").not());

    fs::write(
        project.join("src/typo.ids"),
        "use std::htp::Request\n\nstruct T {\n    r: Request\n}\n",
    )
    .unwrap();
    comline_cmd()
        .current_dir(&project)
        .arg("check")
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "std has no schema matching 'std::htp::Request' - did you mean 'std::http'?",
        ));
}
