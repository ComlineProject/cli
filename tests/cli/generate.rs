//! `comline generate`

use std::fs;

use predicates::prelude::*;

use crate::util::*;

#[test]
fn writes_one_file_per_schema_under_the_default_layout() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success()
        .stderr(predicate::str::contains("language: rust, version:"))
        // Drawn as a tree, the same way `--dry-run` reports.
        .stderr(predicate::str::contains("generated/rust/"))
        .stderr(predicate::str::contains("├─ api.rs").or(predicate::str::contains("└─ api.rs")))
        .stderr(predicate::str::contains("Generated"));

    // default layout: generated/{language}/{namespace}.{ext}
    assert!(project.join("generated/rust/api.rs").exists());
    assert!(project.join("generated/rust/other.rs").exists());
    // and it does NOT freeze a version
    assert!(!project.join(".comline").exists());
}

#[test]
fn honours_out_flag_and_a_flat_layout() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args([
            "generate",
            "--target",
            "rust",
            "--out",
            "bindings",
            "--layout",
            "{{namespace}}.{{ext}}",
        ])
        .assert()
        .success();

    assert!(project.join("bindings/api.rs").exists());
    assert!(project.join("bindings/other.rs").exists());
    assert!(!project.join("generated").exists());
}

#[test]
fn reads_out_from_comline_toml() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    fs::write(project.join("comline.toml"), "[generate]\nout = \"gen\"\n").unwrap();

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success();

    assert!(project.join("gen/rust/api.rs").exists());
}

#[test]
fn env_out_overrides_the_file() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    fs::write(project.join("comline.toml"), "[generate]\nout = \"gen\"\n").unwrap();

    comline_cmd()
        .current_dir(&project)
        .env("COMLINE_GENERATE_OUT", "envgen")
        .args(["generate", "--target", "rust"])
        .assert()
        .success();

    assert!(project.join("envgen/rust/api.rs").exists());
    assert!(!project.join("gen").exists());
}

#[test]
fn out_flag_beats_env_out() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .env("COMLINE_GENERATE_OUT", "envgen")
        .args(["generate", "--target", "rust", "--out", "flaggen"])
        .assert()
        .success();

    assert!(project.join("flaggen/rust/api.rs").exists());
    assert!(!project.join("envgen").exists());
}

#[test]
fn env_out_applies_in_a_multi_target_config() {
    // `--out` alone would error here (see out_flag_needs_target_when_several…);
    // the env var does not — it applies to every target.
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    declare_two_languages(&project);
    fs::write(
        project.join("comline.toml"),
        "[[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\n\n\
         [[generate.target]]\nlanguage = \"python\"\nlang_version = \"3.11.0\"\n",
    )
    .unwrap();

    comline_cmd()
        .current_dir(&project)
        .env("COMLINE_GENERATE_OUT", "envgen")
        .args(["generate", "--target", "rust"])
        .assert()
        .success();

    assert!(project.join("envgen/rust/api.rs").exists());
}

#[test]
fn plain_uses_ascii_output() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    // The default tree, drawn with ASCII branches.
    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust", "--plain"])
        .assert()
        .success()
        .stderr(predicate::str::contains("|- api.rs").or(predicate::str::contains("\\- api.rs")))
        .stderr(predicate::str::contains('├').not())
        .stderr(predicate::str::contains('⚙').not());

    // And `--list`'s arrow.
    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust", "--plain", "--list"])
        .assert()
        .success()
        .stderr(predicate::str::contains("api.ids -> generated/rust/api.rs"))
        .stderr(predicate::str::contains('→').not());
}

#[test]
fn out_flag_needs_target_when_several_are_configured() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    declare_two_languages(&project);
    fs::write(
        project.join("comline.toml"),
        "[[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\n\n\
         [[generate.target]]\nlanguage = \"python\"\nlang_version = \"3.11.0\"\n",
    )
    .unwrap();

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--out", "x"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("need `--target"));
}

#[test]
fn rejects_an_unconfigured_target() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "python"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains(
            "no code-generation target matches `python`",
        ));
}

// -------- lib mode --------

/// Appends `line` to the project's `config.idp`.
fn add_to_config(project: &std::path::Path, line: &str) {
    let path = project.join("config.idp");
    let config = fs::read_to_string(&path).unwrap();
    fs::write(&path, format!("{config}\n{line}\n")).unwrap();
}

#[test]
fn config_idp_wire_framing_reaches_the_rust_generator() {
    let temp = tempfile::tempdir().unwrap();
    let project = copy_fixture("chat_project", temp.path());
    add_to_config(&project, "wire = { framing = \"jsonrpc\" }");

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success();

    // `protocol Chat` has no `@framing` of its own, so it picks up the
    // package's: the JSON-RPC stack, not the datagram one.
    let chat = fs::read_to_string(project.join("generated/rust/chat.rs")).unwrap();
    assert!(
        chat.contains("comline_runtime::framing::JsonRpcFraming"),
        "{chat}"
    );
    assert!(chat.contains("Client::connect_with_framing"), "{chat}");
}

#[test]
fn config_idp_wire_format_reaches_the_rust_generator() {
    let temp = tempfile::tempdir().unwrap();
    let project = copy_fixture("chat_project", temp.path());
    add_to_config(&project, "wire = { format = \"json@1\" }");

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success();

    let chat = fs::read_to_string(project.join("generated/rust/chat.rs")).unwrap();
    assert!(
        chat.contains("pub type Format = comline_runtime::format::Json;"),
        "{chat}"
    );
}

#[test]
fn without_a_wire_key_a_protocol_is_on_a_packed_connection() {
    let temp = tempfile::tempdir().unwrap();
    let project = copy_fixture("chat_project", temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success();

    let chat = fs::read_to_string(project.join("generated/rust/chat.rs")).unwrap();
    assert!(
        chat.contains("pub type Format = comline_runtime::format::Packed;"),
        "{chat}"
    );
    assert!(
        chat.contains("pub struct ChatDispatcher<S>(pub S);"),
        "{chat}"
    );
    assert!(!chat.contains("JsonRpcFraming"), "{chat}");
    // The package's `Services` and `serve`, beside the schemas.
    let connection = fs::read_to_string(project.join("generated/rust/connection.rs")).unwrap();
    assert!(connection.contains("pub struct Services<"), "{connection}");
}

#[test]
fn comline_toml_default_framing_says_it_moved() {
    let temp = tempfile::tempdir().unwrap();
    let project = copy_fixture("chat_project", temp.path());
    fs::write(
        project.join("comline.toml"),
        "[generate]\ndefault_framing = \"jsonrpc\"\n",
    )
    .unwrap();

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "`default_framing` moved from comline.toml to config.idp",
        ))
        .stderr(predicate::str::contains("wire = { framing = \"jsonrpc\" }"));
}

#[test]
fn lib_mode_flatten_skips_the_language_directory() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    fs::write(
        project.join("comline.toml"),
        "[[generate.target]]\n\
         language = \"rust\"\n\
         mode = \"lib\"\n\
         out = \"crate-out\"\n\
         flatten = true\n",
    )
    .unwrap();

    comline_cmd()
        .current_dir(&project)
        .args(["generate"])
        .assert()
        .success();

    let crate_dir = project.join("crate-out");
    assert!(crate_dir.join("Cargo.toml").exists());
    assert!(crate_dir.join("src/lib.rs").exists());
    // not nested under an extra `rust/`
    assert!(!crate_dir.join("rust").exists());
}

#[test]
fn lib_mode_warns_that_layout_has_no_effect() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    fs::write(
        project.join("comline.toml"),
        "[[generate.target]]\n\
         language = \"rust\"\n\
         mode = \"lib\"\n\
         layout = \"{{namespace}}.{{ext}}\"\n",
    )
    .unwrap();

    comline_cmd()
        .current_dir(&project)
        .args(["generate"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "`layout` (`{{namespace}}.{{ext}}`) has no effect in `lib` mode",
        ));
}

#[test]
fn dylib_mode_is_rejected() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust", "--mode", "dylib"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("mode `dylib` is not supported"));
}

// -------- multi-version (package_versions) --------

/// Release twice with a schema change between, so the CAS chain has the
/// releases 0.1.0 and 0.1.1 (a new struct is a minor: before 1.0, the patch).
fn built_twice(project: &std::path::Path) {
    comline_cmd()
        .current_dir(project)
        .arg("release")
        .assert()
        .success();
    let schema = project.join("src/api.ids");
    let mut text = fs::read_to_string(&schema).unwrap();
    text.push_str("\nstruct Added {\n    x: string\n}\n");
    fs::write(&schema, text).unwrap();
    comline_cmd()
        .current_dir(project)
        .arg("release")
        .assert()
        .success();
}

#[test]
fn latest_stamps_the_last_committed_version() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    comline_cmd()
        .current_dir(&project)
        .arg("build")
        .assert()
        .success();
    fs::write(
        project.join("comline.toml"),
        "[generate]\nlayout = \"{{language}}/{{package_version}}/{{namespace}}.{{ext}}\"\n",
    )
    .unwrap();

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success();

    assert!(project.join("generated/rust/0.1.0-dev.1/api.rs").exists());
}

/// The tree shows where files land; only `--list` shows which schema each
/// came from, which is the reason to keep it.
#[test]
fn list_names_the_schema_each_file_came_from() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust", "--list"])
        .assert()
        .success()
        .stderr(predicate::str::contains("api.ids → generated/rust/api.rs"))
        .stderr(predicate::str::contains(
            "other.ids → generated/rust/other.rs",
        ))
        .stderr(predicate::str::contains('├').not());

    assert!(project.join("generated/rust/api.rs").exists());
}

/// `--dry-run` and `--list` are independent: one decides whether anything is
/// written, the other how it is reported.
#[test]
fn dry_run_and_list_combine() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust", "--dry-run", "--list"])
        .assert()
        .success()
        .stderr(predicate::str::contains("api.ids → generated/rust/api.rs"))
        .stderr(predicate::str::contains('├').not())
        .stderr(predicate::str::contains("Would write"));

    assert!(
        !project.join("generated").exists(),
        "a dry run writes nothing"
    );
}

#[test]
fn package_versions_all_generates_every_release() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    built_twice(&project);
    fs::write(
        project.join("comline.toml"),
        "[generate]\npackage_versions = \"all\"\n\
         layout = \"{{language}}/{{package_version}}/{{namespace}}.{{ext}}\"\n",
    )
    .unwrap();

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success()
        // One tree per version. `layout` has to carry `{{package_version}}`
        // for multi-version output, so each tree's root names its version -
        // no `[0.1.0]` tag needed to tell them apart.
        .stderr(predicate::str::contains("generated/rust/0.1.0/"))
        .stderr(predicate::str::contains("generated/rust/0.1.1/"));

    assert!(project.join("generated/rust/0.1.0/api.rs").exists());
    assert!(project.join("generated/rust/0.1.1/api.rs").exists());
    // Releases only: not the builds that led to them.
    assert!(!project.join("generated/rust/0.1.0-dev.1").exists());

    // `--list` tags each line with the version it belongs to.
    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust", "--list"])
        .assert()
        .success()
        .stderr(predicate::str::contains("[0.1.0]"))
        .stderr(predicate::str::contains("[0.1.1]"));
}

#[test]
fn package_versions_list_generates_only_the_named_versions() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    built_twice(&project);
    fs::write(
        project.join("comline.toml"),
        "[generate]\npackage_versions = [\"0.1.0\"]\n\
         layout = \"{{language}}/{{package_version}}/{{namespace}}.{{ext}}\"\n",
    )
    .unwrap();

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success();

    assert!(project.join("generated/rust/0.1.0/api.rs").exists());
    assert!(!project.join("generated/rust/0.1.1").exists());
}

#[test]
fn multiple_versions_need_package_version_in_the_layout() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    built_twice(&project);
    fs::write(
        project.join("comline.toml"),
        "[generate]\npackage_versions = \"all\"\n",
    )
    .unwrap();

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("package_version"));
}

#[test]
fn package_versions_all_without_a_build_errors() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    fs::write(
        project.join("comline.toml"),
        "[generate]\npackage_versions = \"all\"\n\
         layout = \"{{package_version}}/{{namespace}}.{{ext}}\"\n",
    )
    .unwrap();

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("run `comline build` first"));
}

/// Declare a second language in the congregation, so a test can configure
/// two targets. A `[[generate.target]]` only configures a language
/// `config.idp` declares — it can't introduce one.
fn declare_two_languages(project: &std::path::Path) {
    fs::write(
        project.join("config.idp"),
        "congregation simple_project\nspecification_version = 1\n\n\
         code_generation = {\n    languages = {\n        rust#1.85.0 = {}\n\
         \x20       python#3.11.0 = {}\n    }\n}\n",
    )
    .unwrap();
}

fn add_std_reference(project: &std::path::Path) {
    fs::write(
        project.join("src/uses_std.ids"),
        "use std::http::Request\n\nstruct Exchange {\n    request: Request\n}\n",
    )
    .unwrap();
}

#[test]
fn external_std_flag_qualifies_the_reference_and_omits_stds_own_file() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    add_std_reference(&project);

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust", "--external-std"])
        .assert()
        .success();

    let src = fs::read_to_string(project.join("generated/rust/uses_std.rs")).unwrap();
    assert!(
        src.contains("pub request: comline_std::http::Request,"),
        "got: {src}"
    );
    assert!(
        src.contains("// comline_std = { git ="),
        "code mode has no manifest to add the dependency to: {src}"
    );
    assert!(!project.join("generated/rust/std/http.rs").exists());
}

#[test]
fn external_std_off_by_default_still_inlines_std() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    add_std_reference(&project);

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success();

    let src = fs::read_to_string(project.join("generated/rust/uses_std.rs")).unwrap();
    assert!(
        src.contains("pub request: crate::std::http::Request,"),
        "got: {src}"
    );
    assert!(project.join("generated/rust/std/http.rs").exists());
}

#[test]
fn external_std_lib_mode_adds_the_comline_std_dependency() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    add_std_reference(&project);

    comline_cmd()
        .current_dir(&project)
        .args([
            "generate",
            "--target",
            "rust",
            "--mode",
            "lib",
            "--external-std",
        ])
        .assert()
        .success();

    let crate_dir = project.join("generated/rust");
    let cargo = fs::read_to_string(crate_dir.join("Cargo.toml")).unwrap();
    assert!(cargo.contains("comline_std = { git ="), "got: {cargo}");
    assert!(!crate_dir.join("src/std/http.rs").exists());
}

#[test]
fn comline_toml_external_std_is_honoured_without_the_flag() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    add_std_reference(&project);
    fs::write(
        project.join("comline.toml"),
        "[generate]\nexternal_std = true\n",
    )
    .unwrap();

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success();

    let src = fs::read_to_string(project.join("generated/rust/uses_std.rs")).unwrap();
    assert!(src.contains("comline_std::http::Request"), "got: {src}");
}

#[test]
fn env_external_std_beats_comline_toml() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    add_std_reference(&project);
    fs::write(
        project.join("comline.toml"),
        "[generate]\nexternal_std = false\n",
    )
    .unwrap();

    comline_cmd()
        .current_dir(&project)
        .env("COMLINE_GENERATE_EXTERNAL_STD", "true")
        .args(["generate", "--target", "rust"])
        .assert()
        .success();

    let src = fs::read_to_string(project.join("generated/rust/uses_std.rs")).unwrap();
    assert!(src.contains("comline_std::http::Request"), "got: {src}");
}

/// Which mode a target is being generated as was invisible — the only way
/// to tell `code` from `lib` was to look at what landed.
#[test]
fn the_target_line_names_the_mode() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success()
        .stderr(predicate::str::contains(
            "language: rust, version: 1.85.0, mode: code",
        ));

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust", "--mode", "lib"])
        .assert()
        .success()
        .stderr(predicate::str::contains("mode: lib"));
}

/// The generators are pure and `write_file` is the only side effect, so a
/// dry run is the real run with the write suppressed — what it prints is
/// what would be written, not a prediction of it.
#[test]
fn dry_run_prints_the_tree_and_writes_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust", "--mode", "lib", "--dry-run"])
        .assert()
        .success()
        .stderr(predicate::str::contains("dry run, nothing written"))
        .stderr(predicate::str::contains("Cargo.toml"))
        .stderr(predicate::str::contains("src/"))
        .stderr(predicate::str::contains("api.rs"))
        .stderr(predicate::str::contains("Would write 4 file(s)"));

    assert!(
        !project.join("generated").exists(),
        "a dry run must write nothing"
    );
}

/// `lib` mode writes the package manifest, so it can add `comline_std` to
/// it — every `lib` crate used to carry its own private copy of std
/// instead, drifting from everyone else's.
#[test]
fn lib_mode_links_comline_std_instead_of_inlining_it() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    add_std_reference(&project);

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust", "--mode", "lib"])
        .assert()
        .success();

    let crate_dir = project.join("generated/rust");
    let cargo = fs::read_to_string(crate_dir.join("Cargo.toml")).unwrap();
    assert!(cargo.contains("comline_std = { git"), "{cargo}");
    assert!(cargo.contains("rust-version = \"1.85.0\""), "{cargo}");
    assert!(
        !crate_dir.join("src/std.rs").exists(),
        "std should not be inlined"
    );
    assert!(
        !crate_dir.join("src/std").exists(),
        "std should not be inlined"
    );
}

/// `code` mode has no manifest to add a dependency to, so it keeps
/// inlining — and `--no-external-std` puts `lib` back to that too.
#[test]
fn code_mode_still_inlines_std_and_the_flag_overrides_lib() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    add_std_reference(&project);

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success();
    assert!(
        project.join("generated/rust/std/http.rs").exists(),
        "code mode inlines std"
    );

    comline_cmd()
        .current_dir(&project)
        .args([
            "generate",
            "--target",
            "rust",
            "--mode",
            "lib",
            "--no-external-std",
        ])
        .assert()
        .success();
    assert!(
        project.join("generated/rust/src/std/http.rs").exists(),
        "--no-external-std inlines"
    );
}

/// A generator that ignores a feature the schema uses still generates, and
/// says so: TypeScript gets `@validators`' types, not the checks.
#[test]
fn an_ignored_feature_is_a_warning() {
    let temp = tempfile::tempdir().unwrap();
    let project = copy_fixture("validated_project", temp.path());
    let config = project.join("config.idp");
    let idp = std::fs::read_to_string(&config).unwrap();
    std::fs::write(
        &config,
        idp.replace(
            "rust#1.85.0 = {}",
            "rust#1.85.0 = {}\n        typescript#5.0 = {}",
        ),
    )
    .unwrap();

    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "typescript"])
        .assert()
        .success()
        .stderr(predicate::str::contains("`typescript` ignores validators: the types are generated; the checks aren't run yet (core#96)"))
        .stderr(predicate::str::contains("used by schema 'accounts', type alias 'Handle'"));
}

/// `async = true` in `comline.toml`: a protocol on a connection is generated
/// for the async driver; without it, the blocking one.
#[test]
fn async_generates_connection_protocols_for_the_async_driver() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    fs::write(
        project.join("src/link.ids"),
        "protocol Link {\n    function send(n: u32) -> u32;\n}\n",
    )
    .unwrap();
    // On a connection by default.
    let generate = |project: &std::path::Path| {
        comline_cmd()
            .current_dir(project)
            .args(["generate", "--target", "rust"])
            .assert()
            .success();
        fs::read_to_string(project.join("generated/rust/link.rs")).unwrap()
    };

    let blocking = generate(&project);
    assert!(
        blocking.contains("pub struct LinkDispatcher<S>(pub S);"),
        "{blocking}"
    );
    assert!(!blocking.contains("Async"), "{blocking}");

    fs::write(project.join("comline.toml"), "[generate]\nasync = true\n").unwrap();
    let asynchronous = generate(&project);
    assert!(
        asynchronous.contains("pub struct LinkAsyncDispatcher<S>(pub S);"),
        "{asynchronous}"
    );
    assert!(
        asynchronous.contains("pub struct LinkAsyncSender<'p, P: AsyncPeer>(pub &'p P);"),
        "{asynchronous}"
    );
    assert!(
        asynchronous.contains("features = [\"async\"]"),
        "the code-mode note names the runtime feature"
    );
}

/// `async = true` with no protocol on a connection changes nothing, so
/// generate says so; with one (the default), it doesn't.
#[test]
fn async_without_a_connection_protocol_warns() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    fs::write(
        project.join("src/link.ids"),
        "protocol Link {\n    function send(n: u32) -> u32;\n}\n",
    )
    .unwrap();
    let warning = "`async = true`, but no protocol runs on a connection";
    fs::write(project.join("comline.toml"), "[generate]\nasync = true\n").unwrap();
    let config = fs::read_to_string(project.join("config.idp")).unwrap();

    // Every protocol lockstep: nothing for `async` to change.
    add_to_config(&project, "wire = { framing = \"datagram\" }");
    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success()
        .stderr(predicate::str::contains(warning));

    // The default: on a connection.
    fs::write(project.join("config.idp"), config).unwrap();
    comline_cmd()
        .current_dir(&project)
        .args(["generate", "--target", "rust"])
        .assert()
        .success()
        .stderr(predicate::str::contains(warning).not());
}
