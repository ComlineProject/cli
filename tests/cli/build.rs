//! `comline build`

use predicates::prelude::*;

use crate::util::*;

#[test]
fn freezes_an_initial_version() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success()
        .stderr(predicate::str::contains("Project built"))
        .stderr(predicate::str::contains("initial build 0.1.0-dev.1"))
        .stderr(predicate::str::contains(
            "next release: 0.1.0 (the first release)",
        ));

    assert!(project.join(".comline").is_dir());
}

/// `fixture_project`, declaring `languages` (and anything after) instead.
fn declaring(temp: &std::path::Path, languages: &str, rest: &str) -> std::path::PathBuf {
    let project = fixture_project(temp);
    std::fs::write(
        project.join("config.idp"),
        format!(
            "congregation simple_project\nspecification_version = 1\n\n\
             code_generation = {{\n    languages = {{\n        {languages}\n    }}\n}}\n{rest}"
        ),
    )
    .unwrap();
    project
}

fn build(project: &std::path::Path) -> assert_cmd::assert::Assert {
    comline_cmd().current_dir(project).arg("build").assert()
}

#[test]
fn refuses_a_declaration_below_the_generators_floor_and_freezes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let project = declaring(temp.path(), "rust#1.70.0 = {}", "");

    build(&project).failure().stderr(predicate::str::contains(
        "`rust#1.70.0` is older than the rust generator handles: it needs `rust#1.85.0` or later",
    ));
    assert!(!project.join(".comline").exists(), "nothing was frozen");
}

#[test]
fn refuses_a_language_this_build_cant_check() {
    let temp = tempfile::tempdir().unwrap();
    let project = declaring(temp.path(), "python#3.11 = {}", "");

    build(&project).failure().stderr(predicate::str::contains(
        "`python#3.11` can't be checked: this build of comline has no python generator",
    ));
}

#[test]
fn refuses_a_feature_the_declared_target_doesnt_support() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    std::fs::write(
        project.join("src/either.ids"),
        "struct Either {\n    value: union(string u32)\n}\n",
    )
    .unwrap();

    build(&project)
        .failure()
        .stderr(predicate::str::contains(
            "`rust#1.85.0`: the rust generator doesn't support unions",
        ))
        .stderr(predicate::str::contains("struct 'Either'"));
}

#[test]
fn a_version_past_the_verified_range_warns_and_settings_can_make_it_an_error() {
    let temp = tempfile::tempdir().unwrap();
    let project = declaring(temp.path(), "typescript#6.0 = {}", "");
    build(&project)
        .success()
        .stderr(predicate::str::contains(
            "`typescript#6.0` is past what the typescript generator has verified (up to 5.9)",
        ))
        .stderr(predicate::str::contains("generation::unverified-version"));

    let temp = tempfile::tempdir().unwrap();
    let project = declaring(
        temp.path(),
        "typescript#6.0 = {}",
        "\nsettings = {\n    diagnostics = {\n        generation::unverified-version = {\n            level = \"error\"\n        }\n    }\n}\n",
    );
    build(&project)
        .failure()
        .stderr(predicate::str::contains("makes it an error"));
    assert!(!project.join(".comline").exists());
}

#[test]
fn refuses_a_declaration_below_a_dependencys() {
    let temp = tempfile::tempdir().unwrap();
    let shared = temp.path().join("shared");
    std::fs::create_dir_all(shared.join("src")).unwrap();
    std::fs::write(
        shared.join("config.idp"),
        "congregation shared\nspecification_version = 1\n\n\
         code_generation = {\n    languages = {\n        rust#1.90.0 = {}\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(
        shared.join("src/models.ids"),
        "struct Thing {\n    id: u64\n}\n",
    )
    .unwrap();
    let project = declaring(
        temp.path(),
        "rust#1.85.0 = {}",
        "\ndependencies = {\n    shared = {\n        path = \"../shared\"\n    }\n}\n",
    );

    build(&project).failure().stderr(predicate::str::contains(
        "`rust#1.85.0` is older than dependency `shared` allows: it declares `rust#1.90.0`",
    ));
}
