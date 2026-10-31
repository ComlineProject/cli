//! `comline conformance --impl`: implementations from
//! `tests/fixtures/conformance` (built here) run against the official
//! corpus, at the runtime rev the CLI pins, and against a package's own
//! vectors.

use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::OnceLock;

use predicates::prelude::*;

use crate::util::*;

/// The fixture implementations' release directory.
fn built() -> &'static PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let manifest =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/conformance/Cargo.toml");
        let target_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("conformance");
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let status = StdCommand::new(&cargo)
            .args(["build", "--release", "--manifest-path"])
            .arg(&manifest)
            .arg("--target-dir")
            .arg(&target_dir)
            .status()
            .expect("cargo runs");
        assert!(
            status.success(),
            "building the fixture implementations failed"
        );
        target_dir.join("release")
    })
}

fn implementation(bin: &str) -> String {
    built()
        .join(format!("{bin}{}", std::env::consts::EXE_SUFFIX))
        .to_str()
        .unwrap()
        .to_string()
}

/// The runtime corpus the CLI's `comline-runtime-conformance` comes with:
/// that crate's directory, from `cargo metadata`.
fn corpus() -> &'static PathBuf {
    static CORPUS: OnceLock<PathBuf> = OnceLock::new();
    CORPUS.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let output = StdCommand::new(cargo)
            .args(["metadata", "--format-version", "1", "--manifest-path"])
            .arg(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .output()
            .expect("cargo metadata runs");
        assert!(output.status.success());
        let metadata: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let manifest = metadata["packages"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "comline-runtime-conformance")
            .expect("comline-runtime-conformance is a dependency")["manifest_path"]
            .as_str()
            .unwrap()
            .to_string();
        Path::new(&manifest).parent().unwrap().to_path_buf()
    })
}

#[test]
fn the_reference_passes_every_official_encoding() {
    comline_cmd()
        .args([
            "conformance",
            "--impl",
            &implementation("reference"),
            "--corpus",
        ])
        .arg(corpus())
        .assert()
        .success()
        .stderr(predicate::str::contains("packed@1 (format): "))
        .stderr(predicate::str::contains("failed").not())
        .stderr(predicate::str::contains("unproven").not())
        .stderr(predicate::str::contains(
            "proven  struct_fields: by_position",
        ))
        .stderr(predicate::str::contains("conforms to every encoding"));
}

#[test]
fn a_flipped_byte_fails_with_a_diff() {
    comline_cmd()
        .args(["conformance", "--impl", &implementation("flipper")])
        .args(["--encoding", "datagram@1", "--corpus"])
        .arg(corpus())
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "unproven  calls: by_index — case `framing-datagram/request` failed",
        ))
        .stderr(predicate::str::contains(
            "framing-datagram/request failed: encodes wrong",
        ))
        .stderr(predicate::str::contains("expected: 03002a"))
        .stderr(predicate::str::contains("got: fc002a"))
        .stderr(predicate::str::contains("^^ byte 0 differs"));
}

/// A package declaring a namespaced framing, with its vectors beside its
/// profile, and a stub implementing it.
#[test]
fn a_package_proves_its_own_framing() {
    let temp = tempfile::tempdir().unwrap();
    let project = copy_fixture("conformance_package", temp.path());
    copy_dir_all(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/conformance_package/profiles"),
        &project.join("profiles"),
    );

    comline_cmd()
        .current_dir(&project)
        .args(["conformance", "--impl", &implementation("lines")])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "com.acme.lines@1 (framing): 5/5 case(s) pass",
        ))
        .stderr(predicate::str::contains("proven  calls: by_name"))
        .stderr(predicate::str::contains("proven  errors: by_ordinal"));

    // The reference doesn't implement it: every case is not implemented.
    comline_cmd()
        .current_dir(&project)
        .args(["conformance", "--impl", &implementation("reference")])
        .args(["--encoding", "com.acme.lines@1"])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("doesn't list `com.acme.lines@1`"))
        .stderr(predicate::str::contains("lines/request not implemented"));

    // Without vectors beside the profile, it says where they belong.
    std::fs::remove_file(project.join("profiles/lines.vectors.json")).unwrap();
    comline_cmd()
        .current_dir(&project)
        .args(["conformance", "--impl", &implementation("lines")])
        .assert()
        .code(1)
        .stderr(predicate::str::contains(
            "lines.vectors.json beside its profile",
        ));
}

#[test]
fn an_official_name_points_at_the_corpus() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    comline_cmd()
        .current_dir(&project)
        .args([
            "conformance",
            "--impl",
            "/bin/true",
            "--encoding",
            "packed@1",
        ])
        .assert()
        .code(1)
        .stderr(predicate::str::contains("doesn't declare `packed@1`"))
        .stderr(predicate::str::contains("--corpus"));
}

fn copy_dir_all(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        std::fs::copy(entry.path(), to.join(entry.file_name())).unwrap();
    }
}
