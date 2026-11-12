//! `comline deps remove`

use std::fs;

use predicates::prelude::*;

use crate::util::*;

fn comline(project: &std::path::Path, args: &[&str]) -> assert_cmd::assert::Assert {
    comline_cmd().current_dir(project).args(args).assert()
}

#[test]
fn removes_an_unused_dependency() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_sibling(temp.path());
    comline(&project, &["deps", "add", "shared", "../shared"]).success();
    let with_it = fs::read_to_string(project.join("config.idp")).unwrap();
    assert!(with_it.contains("shared = {"), "{with_it}");

    comline(&project, &["deps", "remove", "shared"])
        .success()
        .stderr(predicate::str::contains("Removed shared from config.idp"));

    // Only its entry went: the block it was in and the rest stay as written.
    let after = fs::read_to_string(project.join("config.idp")).unwrap();
    let (before, _) = with_it.split_once("dependencies = {").unwrap();
    assert_eq!(after, format!("{before}dependencies = {{\n}}\n"));
    comline(&project, &["check"]).success();
}

#[test]
fn only_its_own_entry_goes() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_sibling(temp.path());
    write_package(&temp.path().join("extra"), "extra");
    comline(&project, &["deps", "add", "shared", "../shared"]).success();
    comline(&project, &["deps", "add", "extra", "../extra"]).success();
    let config = fs::read_to_string(project.join("config.idp")).unwrap();
    let extra_entry = config.split_once("    extra = {").unwrap().1;

    comline(&project, &["deps", "remove", "shared"]).success();
    let after = fs::read_to_string(project.join("config.idp")).unwrap();
    assert!(!after.contains("shared"), "{after}");
    assert!(
        after.ends_with(&format!("dependencies = {{\n    extra = {{{extra_entry}")),
        "{after}"
    );
    comline(&project, &["deps", "list"])
        .success()
        .stdout(predicate::str::contains("extra"))
        .stdout(predicate::str::contains("shared").not());
}

#[test]
fn refuses_one_a_schema_still_imports_and_names_the_use() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_sibling(temp.path());
    comline(&project, &["deps", "add", "shared", "../shared"]).success();
    uses_thing_from(&project, "shared");
    let before = fs::read_to_string(project.join("config.idp")).unwrap();

    comline(&project, &["deps", "remove", "shared"])
        .failure()
        .code(1)
        .stderr(predicate::str::contains("can't remove `shared`"))
        .stderr(predicate::str::contains(
            "src/holder.ids:1  use shared::models::Thing",
        ));
    // Not even as a dry run.
    comline(&project, &["deps", "remove", "shared", "--dry-run"])
        .failure()
        .stderr(predicate::str::contains("src/holder.ids:1"));
    assert_eq!(
        fs::read_to_string(project.join("config.idp")).unwrap(),
        before
    );

    // Once nothing imports it, it goes.
    fs::remove_file(project.join("src/holder.ids")).unwrap();
    comline(&project, &["deps", "remove", "shared"]).success();
}

#[test]
fn every_use_is_listed_with_its_file_and_line() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_sibling(temp.path());
    comline(&project, &["deps", "add", "shared", "../shared"]).success();
    fs::create_dir_all(project.join("src/nested")).unwrap();
    fs::write(
        project.join("src/holder.ids"),
        "// use shared::models::Thing\n\nuse shared::models::*\n\nstruct Holder {\n    thing: Thing\n}\n",
    )
    .unwrap();
    fs::write(
        project.join("src/nested/more.ids"),
        "use shared::models::{Thing}\n\nstruct More {\n    thing: Thing\n}\n",
    )
    .unwrap();

    comline(&project, &["deps", "remove", "shared"])
        .failure()
        .stderr(predicate::str::contains(
            "2 schema statements still import it",
        ))
        .stderr(predicate::str::contains(
            "src/holder.ids:3  use shared::models::*",
        ))
        .stderr(predicate::str::contains(
            "src/nested/more.ids:1  use shared::models::{Thing}",
        ))
        .stderr(predicate::str::contains("src/holder.ids:1").not());
}

#[test]
fn dry_run_writes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_sibling(temp.path());
    comline(&project, &["deps", "add", "shared", "../shared"]).success();
    let before = fs::read_to_string(project.join("config.idp")).unwrap();

    comline(&project, &["deps", "remove", "shared", "--dry-run"])
        .success()
        .stderr(predicate::str::contains(
            "Would remove shared from config.idp",
        ))
        .stderr(predicate::str::contains("path = \"../shared\""))
        .stderr(predicate::str::contains("Removed").not());
    assert_eq!(
        fs::read_to_string(project.join("config.idp")).unwrap(),
        before
    );
}

#[test]
fn an_unknown_name_lists_the_declared_dependencies() {
    let temp = tempfile::tempdir().unwrap();
    let project = project_with_sibling(temp.path());
    comline(&project, &["deps", "remove", "shared"])
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "`shared` isn't a dependency in config.idp; it declares none",
        ));

    comline(&project, &["deps", "add", "shared", "../shared"]).success();
    let before = fs::read_to_string(project.join("config.idp")).unwrap();
    comline(&project, &["deps", "remove", "nope"])
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "`nope` isn't a dependency in config.idp; it declares: shared",
        ));
    assert_eq!(
        fs::read_to_string(project.join("config.idp")).unwrap(),
        before
    );
}
