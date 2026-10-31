//! `generator = "<path>"`: external generators, native and `.wasm`.
//!
//! The generators come from `tests/fixtures/generators`, built once per run
//! for the host and for `wasm32-wasip1` (which must be installed:
//! `rustup target add wasm32-wasip1`). `real` is the official Rust generator
//! behind the generator protocol; `hostile` misbehaves in the way its file
//! name says.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as StdCommand;
use std::sync::OnceLock;

use predicates::prelude::*;

use crate::util::*;

/// The fixture generators' build directories: (native, wasm32-wasip1).
fn built() -> &'static (PathBuf, PathBuf) {
    static BUILT: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();
    BUILT.get_or_init(|| {
        let manifest =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/generators/Cargo.toml");
        let target_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("generators");
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        for target in [None, Some("wasm32-wasip1")] {
            let mut build = StdCommand::new(&cargo);
            build
                .args(["build", "--release", "--manifest-path"])
                .arg(&manifest)
                .arg("--target-dir")
                .arg(&target_dir);
            if let Some(target) = target {
                build.args(["--target", target]);
            }
            let status = build.status().expect("cargo runs");
            assert!(
                status.success(),
                "building the fixture generators ({target:?}) failed"
            );
        }
        (
            target_dir.join("release"),
            target_dir.join("wasm32-wasip1/release"),
        )
    })
}

#[derive(Clone, Copy, Debug)]
enum Kind {
    Native,
    Wasm,
}

/// Copies fixture binary `bin` into `project` as `name` (plus the kind's
/// extension), returning the file name to put in `comline.toml`.
fn install(project: &Path, kind: Kind, bin: &str, name: &str) -> String {
    let (native, wasm) = built();
    let (from, file) = match kind {
        Kind::Native => (
            native.join(format!("{bin}{}", std::env::consts::EXE_SUFFIX)),
            format!("{name}{}", std::env::consts::EXE_SUFFIX),
        ),
        Kind::Wasm => (wasm.join(format!("{bin}.wasm")), format!("{name}.wasm")),
    };
    fs::copy(&from, project.join(&file)).unwrap_or_else(|e| panic!("{}: {e}", from.display()));
    file
}

/// Every file under `dir`, relative, with its contents.
fn tree(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        for entry in fs::read_dir(&d).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                out.push((
                    path.strip_prefix(dir).unwrap().to_owned(),
                    fs::read(&path).unwrap(),
                ));
            }
        }
    }
    out.sort();
    out
}

/// The built-in Rust generator's output for the fixture, in `mode`.
fn builtin_output(mode: &str) -> Vec<(PathBuf, Vec<u8>)> {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--mode", mode])
        .assert()
        .success();
    tree(&project.join("generated"))
}

fn matches_builtin(kind: Kind) {
    for mode in ["code", "lib"] {
        let temp = tempfile::tempdir().unwrap();
        let project = fixture_project(temp.path());
        let file = install(&project, kind, "real", "comline-gen-rust");
        // No `language`: the generator offers only `rust`.
        fs::write(
            project.join("comline.toml"),
            format!("[[generate.target]]\ngenerator = \"{file}\"\nmode = \"{mode}\"\n"),
        )
        .unwrap();
        comline_cmd()
            .current_dir(&project)
            .arg("generate")
            .assert()
            .success();
        let external = tree(&project.join("generated"));
        assert!(!external.is_empty());
        assert!(
            external == builtin_output(mode),
            "{kind:?} {mode}: the external output differs from the built-in's"
        );
    }
}

#[cfg(feature = "gen-wasm")]
#[test]
fn a_wasm_generator_writes_what_the_builtin_does() {
    matches_builtin(Kind::Wasm);
}

#[test]
fn a_native_generator_writes_what_the_builtin_does() {
    matches_builtin(Kind::Native);
}

/// Runs hostile generator `mode` as `kind`, expecting a failure whose
/// message contains `expected`, and nothing written.
fn refused(kind: Kind, mode: &str, extra: &str, expected: &str) -> String {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    let file = install(&project, kind, "hostile", mode);
    fs::write(
        project.join("comline.toml"),
        format!("[[generate.target]]\nlanguage = \"rust\"\ngenerator = \"{file}\"\n{extra}"),
    )
    .unwrap();
    let output = comline_cmd()
        .current_dir(&project)
        .arg("generate")
        .assert()
        .failure()
        .get_output()
        .clone();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    assert!(
        stderr.contains(expected),
        "{kind:?} {mode}: expected `{expected}` in:\n{stderr}"
    );
    assert!(
        stderr.contains(&format!("generator `{file}`")),
        "{kind:?} {mode}: the generator isn't named:\n{stderr}"
    );
    assert!(
        !project.join("generated").exists(),
        "{kind:?} {mode}: something was written"
    );
    assert!(!temp.path().join("escape.rs").exists() && !project.join("escape.rs").exists());
    stderr
}

/// Runs `test` for a `.wasm` (when this build runs them) and a native
/// generator.
fn both(test: impl Fn(Kind)) {
    if cfg!(feature = "gen-wasm") {
        test(Kind::Wasm);
    }
    test(Kind::Native);
}

#[test]
fn a_parent_path_is_refused() {
    both(|kind| {
        refused(
            kind,
            "dotdot",
            "",
            "returned the path `../escape.rs`, which isn't normalized",
        );
    });
}

#[test]
fn an_absolute_path_is_refused() {
    both(|kind| {
        refused(
            kind,
            "absolute",
            "",
            "which is absolute; nothing was written",
        );
        assert!(!Path::new("/tmp/comline-hostile-absolute.rs").exists());
    });
}

#[test]
fn oversized_output_is_refused() {
    both(|kind| {
        refused(
            kind,
            "oversized",
            "",
            "its output passed the 268435456-byte limit",
        );
    });
}

#[test]
fn a_generator_that_never_finishes_is_stopped() {
    both(|kind| {
        refused(
            kind,
            "timeout",
            "timeout = 2\n",
            "didn't finish within 2s and was stopped",
        );
    });
}

#[test]
fn a_protocol_mismatch_names_both_sides() {
    both(|kind| {
        refused(
            kind,
            "mismatch",
            "",
            "speaks generator protocol 7, and this comline speaks protocol 0",
        );
    });
}

#[cfg(feature = "gen-wasm")]
#[test]
fn a_wasm_generator_cannot_read_the_filesystem() {
    let stderr = refused(Kind::Wasm, "readfs", "", "couldn't read the filesystem");
    assert!(!stderr.contains("simple_project"));
}

#[test]
fn stderr_is_shown_prefixed_and_capped() {
    both(|kind| {
        let stderr = refused(kind, "stderr", "", "refusing after the noise");
        let name = match kind {
            Kind::Wasm => "stderr.wasm".to_string(),
            Kind::Native => format!("stderr{}", std::env::consts::EXE_SUFFIX),
        };
        assert!(
            stderr.contains(&format!("  [{name}] hello from the generator")),
            "{stderr}"
        );
        assert!(
            stderr.contains("(stderr past 65536 bytes not shown)"),
            "{stderr}"
        );
    });
}

#[test]
fn a_language_the_generator_does_not_offer_is_refused() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    let file = install(&project, Kind::Native, "hostile", "future");
    fs::write(
        project.join("comline.toml"),
        format!("[[generate.target]]\ngenerator = \"{file}\"\n"),
    )
    .unwrap();
    comline_cmd()
        .current_dir(&project)
        .arg("generate")
        .assert()
        .failure()
        .stderr(predicate::str::contains(format!(
            "generator `{file}` handles `rust` from `9.0.0` on, not `1.85.0`"
        )));
}
