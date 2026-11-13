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

/// Appends `line` to the project's `config.idp`.
pub fn add_to_config(project: &Path, line: &str) {
    let path = project.join("config.idp");
    let config = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("{config}\n{line}\n")).unwrap();
}

/// The fixture with a protocol that is on no connection (lockstep on the
/// `datagram` framing), so the package has no `serve` and generating says why.
pub fn lockstep_project(temp: &Path) -> PathBuf {
    let project = fixture_project(temp);
    add_to_config(&project, "wire = { framing = \"datagram\" }");
    fs::write(
        project.join("src/link.ids"),
        "protocol Link {\n    function send(n: u32) -> u32;\n}\n",
    )
    .unwrap();
    project
}

/// Like [`fixture_project`] for an arbitrary fixture under `tests/fixtures/`.
pub fn copy_fixture(name: &str, temp: &Path) -> PathBuf {
    let dest = temp.join("proj");
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    fs::create_dir_all(&dest).unwrap();
    fs::copy(src.join("config.idp"), dest.join("config.idp")).unwrap();
    if src.join("comline.toml").exists() {
        fs::copy(src.join("comline.toml"), dest.join("comline.toml")).unwrap();
    }
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

/// The package defaults every string and list takes where its type writes
/// no maximum (bounds on types, core#98), as one `settings` entry.
pub const BOUNDS: &str = "bounds = { string = { max = 256 } array = { max = 1024 } }";

/// `config` (a whole `config.idp`) with the bound defaults added: into its
/// top-level `settings` block if it has one, else as a block of their own.
pub fn with_bounds(config: &str) -> String {
    with_setting(config, BOUNDS)
}

/// `config` with `entry` added to its top-level `settings`, or in a new
/// `settings` block when it has none.
pub fn with_setting(config: &str, entry: &str) -> String {
    match config.find("\nsettings = {") {
        Some(at) => {
            let open = at + "\nsettings = {".len();
            format!("{} {entry}\n{}", &config[..open], &config[open..])
        }
        None if config.starts_with("settings = {") => {
            let open = "settings = {".len();
            format!("{} {entry}\n{}", &config[..open], &config[open..])
        }
        None => {
            let sep = if config.ends_with('\n') { "" } else { "\n" };
            format!("{config}{sep}settings = {{ {entry} }}\n")
        }
    }
}

/// A minimal package at `dir`, declaring `Thing` in `src/models.ids`.
pub fn write_package(dir: &Path, name: &str) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("config.idp"),
        with_bounds(&format!("congregation {name}\nspecification_version = 1\n")),
    )
    .unwrap();
    fs::write(
        dir.join("src/models.ids"),
        "struct Thing {\n    id: u64\n}\n",
    )
    .unwrap();
}

/// The fixture project, with a `shared` package next to it.
pub fn project_with_sibling(temp: &Path) -> PathBuf {
    let project = fixture_project(temp);
    write_package(&temp.join("shared"), "shared");
    project
}

pub fn uses_thing_from(project: &Path, dependency: &str) {
    fs::write(
        project.join("src/holder.ids"),
        format!("use {dependency}::models::Thing\n\nstruct Holder {{\n    thing: Thing\n}}\n"),
    )
    .unwrap();
}
