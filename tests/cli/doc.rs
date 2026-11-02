//! `comline doc`

use std::fs;
use std::path::Path;

use predicates::prelude::*;

use crate::util::*;

fn package(dir: &Path, header: &str, schema: &str) {
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(
        dir.join("config.idp"),
        format!("{header}congregation shop\nspecification_version = 1\n"),
    )
    .unwrap();
    fs::write(dir.join("src/api.ids"), schema).unwrap();
}

const API: &str = "//! The shop's API.\n\n/// One order.\nstruct Order {\n    /// What it costs.\n    total: u64\n}\n\n\
    /// Orders things.\nprotocol Orders {\n    /// Place one.\n    /// @order: what to place\n    \
    function place(order: Order) -> bool;\n}\n";

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn writes_markdown_pages_for_the_working_tree() {
    let temp = tempfile::tempdir().unwrap();
    let shop = temp.path().join("shop");
    package(&shop, "//! A shop.\n\n", API);

    comline_cmd()
        .current_dir(&shop)
        .arg("doc")
        .assert()
        .success()
        .stderr(predicate::str::contains("Documented 1 version(s) of shop"));

    // Nothing built: the unreleased state is the one version.
    let site = shop.join("docs/api/markdown");
    assert!(read(&site.join("index.md")).contains("- [unreleased](unreleased/index.md)"));
    let out = site.join("unreleased");
    let index = read(&out.join("index.md"));
    assert!(index.starts_with("# shop\n\nA shop.\n"), "{index}");
    assert!(index.contains("| [`api`](api.md) | The shop's API. |"));
    let api = read(&out.join("api.md"));
    assert!(
        api.contains("| `total` | `u64` | What it costs. |"),
        "{api}"
    );
    assert!(api.contains("| `order` | [`Order`](#order) | what to place |"));
    assert!(api.contains("**On the wire**"));
    assert!(read(&out.join("nav.yml")).contains("  - \"api\": api.md"));
    assert!(!out.join("changelog.md").exists());
}

#[test]
fn documents_a_built_version_with_its_frozen_headers_and_a_changelog() {
    let temp = tempfile::tempdir().unwrap();
    let shop = temp.path().join("shop");
    package(&shop, "//! A shop.\n\n", API);
    comline_cmd()
        .current_dir(&shop)
        .arg("build")
        .assert()
        .success();

    // Change the working tree: the built version must not read it.
    package(
        &shop,
        "",
        &API.replace("//! The shop's API.\n\n", "").replace(
            "    total: u64\n",
            "    total: u64\n    optional note: string\n",
        ),
    );
    comline_cmd()
        .current_dir(&shop)
        .arg("build")
        .assert()
        .success();

    comline_cmd()
        .current_dir(&shop)
        .args(["doc", "--version", "0.1.0-dev.1", "--out", "v1"])
        .assert()
        .success();
    assert!(!shop.join("v1/markdown/index.md").exists());
    let index = read(&shop.join("v1/markdown/0.1.0-dev.1/index.md"));
    assert!(
        index.starts_with("# shop 0.1.0-dev.1\n\n> **Note:** Built versions don't record")
            && index.contains("\n\nA shop.\n"),
        "{index}"
    );
    let api = read(&shop.join("v1/markdown/0.1.0-dev.1/api.md"));
    assert!(api.starts_with("# `api`\n\n> **Note:**"), "{api}");
    assert!(api.contains("\n\nThe shop's API.\n"), "{api}");
    assert!(!api.contains("note"));

    comline_cmd()
        .current_dir(&shop)
        .args(["doc", "--out", "now"])
        .assert()
        .success();
    // The working tree is the last build: no unreleased state, and each
    // version carries the changelog from the one before it.
    assert!(!shop.join("now/markdown/unreleased").exists());
    assert!(!shop.join("now/markdown/0.1.0-dev.1/changelog.md").exists());
    let changelog = read(&shop.join("now/markdown/0.1.0-dev.2/changelog.md"));
    assert!(
        changelog.contains("## 0.1.0-dev.1 to 0.1.0-dev.2"),
        "{changelog}"
    );
    // The entry names the field, then shows its new IDL line.
    assert!(changelog.contains(".note`"), "{changelog}");
    assert!(changelog.contains("```diff\n+"), "{changelog}");
}

#[test]
fn json_writes_the_model_and_doc_settings_come_from_comline_toml() {
    let temp = tempfile::tempdir().unwrap();
    let shop = temp.path().join("shop");
    package(&shop, "", API);
    fs::write(
        shop.join("comline.toml"),
        "[doc]\nout = \"site\"\nformats = [\"json\"]\n",
    )
    .unwrap();

    comline_cmd()
        .current_dir(&shop)
        .arg("doc")
        .assert()
        .success();
    let model: serde_json::Value =
        serde_json::from_str(&read(&shop.join("site/json/unreleased/model.json"))).unwrap();
    assert_eq!(model["model_version"], 1);
    assert_eq!(model["package"]["name"], "shop");
    assert_eq!(model["modules"][0]["docs"], "The shop's API.");
    let index: serde_json::Value =
        serde_json::from_str(&read(&shop.join("site/json/index.json"))).unwrap();
    assert_eq!(index["latest"], "unreleased");
    assert_eq!(index["versions"][0]["model"], "unreleased/model.json");
}

#[test]
fn an_unknown_format_names_what_is_offered() {
    let temp = tempfile::tempdir().unwrap();
    let shop = temp.path().join("shop");
    package(&shop, "", API);

    comline_cmd()
        .current_dir(&shop)
        .args(["doc", "--format", "pdf"])
        .assert()
        .failure()
        .stderr(predicate::str::contains("doesn't render `pdf`; it offers"))
        .stderr(predicate::str::contains("`html`"))
        .stderr(predicate::str::contains("`markdown`"));
}

#[test]
fn html_writes_a_self_contained_site_per_version() {
    let temp = tempfile::tempdir().unwrap();
    let shop = temp.path().join("shop");
    package(&shop, "//! A shop.\n\n", API);
    comline_cmd()
        .current_dir(&shop)
        .arg("build")
        .assert()
        .success();
    package(
        &shop,
        "//! A shop.\n\n",
        &API.replace(
            "    total: u64\n",
            "    total: u64\n    optional note: string\n",
        ),
    );

    comline_cmd()
        .current_dir(&shop)
        .args(["doc", "--format", "html", "--out", "site"])
        .assert()
        .success();

    let site = shop.join("site/html");
    // The root points at the latest version, which is the dev build here.
    let root = read(&site.join("index.html"));
    assert!(root.contains("url=0.1.0-dev.1/index.html"), "{root}");
    assert!(read(&site.join("versions.js")).contains("var LATEST = \"0.1.0-dev.1\";"));
    let compare = read(&site.join("compare.js"));
    assert!(
        compare.contains("\"api::Order.note\":\"optional string\""),
        "{compare}"
    );
    assert!(site.join("compare.html").exists());

    let out = site.join("unreleased");
    let index = read(&out.join("index.html"));
    assert!(index.starts_with("<!doctype html>"), "{index}");
    assert!(index.contains("<h1>shop</h1>\n<p>A shop.</p>"), "{index}");
    assert!(index.contains("<a href=\"api.html\"><code>api</code></a>"));
    let api = read(&out.join("api.html"));
    assert!(
        api.contains("<td><code>total</code></td><td><code>u64</code></td><td>What it costs.</td>"),
        "{api}"
    );
    assert!(api.contains("<a href=\"#order\"><code>Order</code></a>"));
    assert!(api.contains("<h3>On the wire</h3>"));
    assert!(api.contains("<link rel=\"stylesheet\" href=\"style.css\">"));
    assert!(api.contains("<script src=\"../versions.js\"></script>"));
    assert!(api.contains("<select id=\"version-picker\">"));
    assert!(read(&out.join("search.js")).contains("\"u\":\"api.html#order\""));
    assert!(out.join("style.css").exists());
    assert!(out.join("changelog.html").exists());
    assert!(!out.join("nav.yml").exists());

    // A built version has no wire tables yet, and says so.
    let built = read(&site.join("0.1.0-dev.1/api.html"));
    assert!(built.contains("record their wire tables yet"), "{built}");
    assert!(!built.contains("<h3>On the wire</h3>"));
}

#[test]
fn html_is_a_doc_format_in_comline_toml() {
    let temp = tempfile::tempdir().unwrap();
    let shop = temp.path().join("shop");
    package(&shop, "", API);
    fs::write(
        shop.join("comline.toml"),
        "[doc]\nout = \"site\"\nformats = [\"html\", \"markdown\"]\n",
    )
    .unwrap();
    comline_cmd()
        .current_dir(&shop)
        .arg("doc")
        .assert()
        .success();
    assert!(shop.join("site/html/index.html").exists());
    assert!(shop.join("site/html/unreleased/api.html").exists());
    assert!(shop.join("site/markdown/index.md").exists());
    assert!(shop.join("site/markdown/unreleased/api.md").exists());
}

/// Builds `shop` into two releases and a dev build, then changes the working
/// tree: 0.1.0-dev.1, 0.1.0, 0.2.0-dev.1, 0.2.0, 0.3.0-dev.1 and unreleased.
fn three_versions(shop: &Path) {
    package(shop, "//! A shop.\n\n", API);
    let step = |args: &[&str]| {
        comline_cmd()
            .current_dir(shop)
            .args(args)
            .assert()
            .success();
    };
    step(&["build"]);
    step(&["release", "--yes"]);
    let api = API.replace(
        "    total: u64\n",
        "    total: u64\n    optional note: string\n",
    );
    package(shop, "//! A shop.\n\n", &api);
    step(&["build"]);
    step(&["release", "--yes"]);
    let api = api.replace(
        "    total: u64\n",
        "    total: u64\n    optional tag: string\n",
    );
    package(shop, "//! A shop.\n\n", &api);
    step(&["build"]);
    package(shop, "//! A shop, changed.\n\n", &api);
}

fn folders(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn every_version_is_documented_and_a_second_run_rerenders_only_unreleased() {
    let temp = tempfile::tempdir().unwrap();
    let shop = temp.path().join("shop");
    three_versions(&shop);

    comline_cmd()
        .current_dir(&shop)
        .args(["doc", "--format", "html"])
        .assert()
        .success()
        .stderr(predicate::str::contains("6 rendered, 0 unchanged"));
    let site = shop.join("docs/api/html");
    assert_eq!(
        folders(&site),
        [
            "0.1.0",
            "0.1.0-dev.1",
            "0.2.0",
            "0.2.0-dev.1",
            "0.3.0-dev.1",
            "unreleased"
        ]
    );
    assert!(read(&site.join("index.html")).contains("url=0.2.0/index.html"));
    let versions = read(&site.join("versions.js"));
    assert!(
        versions.contains("{\"id\":\"0.3.0-dev.1\",\"kind\":\"dev\""),
        "{versions}"
    );
    assert!(versions.contains("\"release\":\"0.3.0\""), "{versions}");

    // A marker in a kept folder survives; the unreleased folder is redone.
    fs::write(site.join("0.1.0/marker"), "").unwrap();
    fs::write(site.join("unreleased/marker"), "").unwrap();
    comline_cmd()
        .current_dir(&shop)
        .args(["doc", "--format", "html"])
        .assert()
        .success()
        .stderr(predicate::str::contains("1 rendered, 5 unchanged"));
    assert!(site.join("0.1.0/marker").exists());
    assert!(!site.join("unreleased/marker").exists());
    assert!(read(&site.join("unreleased/index.html")).contains("A shop, changed."));
}

#[test]
fn doc_versions_trims_the_list() {
    let temp = tempfile::tempdir().unwrap();
    let shop = temp.path().join("shop");
    three_versions(&shop);
    let run = |versions: &str| {
        fs::write(
            shop.join("comline.toml"),
            format!("[doc]\nformats = [\"markdown\"]\nversions = {versions}\n"),
        )
        .unwrap();
        comline_cmd()
            .current_dir(&shop)
            .arg("doc")
            .assert()
            .success();
        folders(&shop.join("docs/api/markdown"))
    };

    assert_eq!(run("\"all\"").len(), 6);
    assert_eq!(run("{ last = 2 }"), ["0.3.0-dev.1", "unreleased"]);
    assert_eq!(run("{ releases_only = true }"), ["0.1.0", "0.2.0"]);
    assert_eq!(
        run("{ since = \"0.2.0\" }"),
        ["0.2.0", "0.3.0-dev.1", "unreleased"]
    );
    assert_eq!(
        run("{ since = \"0.1.0\", releases_only = true, last = 1 }"),
        ["0.2.0"]
    );
    let index = read(&shop.join("docs/api/markdown/index.md"));
    assert!(
        index.contains("- [0.2.0](0.2.0/index.md): release"),
        "{index}"
    );

    fs::write(
        shop.join("comline.toml"),
        "[doc]\nversions = { since = \"9.9.9\" }\n",
    )
    .unwrap();
    comline_cmd()
        .current_dir(&shop)
        .arg("doc")
        .assert()
        .failure()
        .stderr(predicate::str::contains("since `9.9.9`: no such version"));
}

/// A release's changelog is everything since the previous release, even
/// when it's built from the same files as its last dev build; a dev build's
/// is its step, with a link to everything since the last release.
#[test]
fn a_release_changelog_covers_everything_since_the_previous_release() {
    let temp = tempfile::tempdir().unwrap();
    let shop = temp.path().join("shop");
    three_versions(&shop);
    comline_cmd()
        .current_dir(&shop)
        .args(["doc", "--format", "markdown"])
        .assert()
        .success();
    let site = shop.join("docs/api/markdown");

    // 0.2.0 is built from 0.2.0-dev.1's files: its step is empty, but its
    // page shows what changed since 0.1.0.
    let release = read(&site.join("0.2.0/changelog.md"));
    assert!(release.contains("## 0.1.0 to 0.2.0\n"), "{release}");
    assert!(release.contains("`Order.note`"), "{release}");
    assert!(!release.contains("0.2.0-dev.1 to 0.2.0"), "{release}");
    assert!(!release.contains("No changes"), "{release}");
    // The dev build keeps its own step.
    let dev = read(&site.join("0.2.0-dev.1/changelog.md"));
    assert!(dev.contains("## 0.1.0 to 0.2.0-dev.1\n"), "{dev}");
    assert!(dev.contains("`Order.note`"), "{dev}");
    // Past the first build after a release, a dev state links to its
    // changes since that release.
    let unreleased = read(&site.join("unreleased/changelog.md"));
    assert!(
        unreleased.contains("## 0.3.0-dev.1 to unreleased\n"),
        "{unreleased}"
    );
    assert!(
        unreleased
            .contains("Everything since the last release: [0.2.0 to unreleased](#since-0.2.0)."),
        "{unreleased}"
    );
    let since = unreleased
        .split("## Since the last release: 0.2.0 to unreleased")
        .nth(1)
        .unwrap();
    assert!(since.contains("`Order.tag`"), "{unreleased}");

    // The compare page still composes the steps alone.
    comline_cmd()
        .current_dir(&shop)
        .args(["doc", "--format", "html"])
        .assert()
        .success();
    let compare = read(&shop.join("docs/api/html/compare.js"));
    assert_eq!(
        compare.matches("\"title\":\"Order.note").count(),
        1,
        "{compare}"
    );
}

/// A dependency's (here `std`'s) schemas are never the package's modules:
/// what it reaches of them is listed apart, linked to std's hosted docs.
#[test]
fn dependencies_are_listed_apart_by_reach() {
    let temp = tempfile::tempdir().unwrap();
    let shop = temp.path().join("shop");
    let api = API.replace(
        "/// One order.\nstruct Order {\n",
        "use std::http::HttpMethod\n\n/// One order.\nstruct Order {\n    method: HttpMethod\n",
    );
    package(&shop, "//! A shop.\n\n", &api);
    comline_cmd()
        .current_dir(&shop)
        .arg("build")
        .assert()
        .success();
    package(&shop, "//! A shop, changed.\n\n", &api);
    comline_cmd()
        .current_dir(&shop)
        .args(["doc", "--format", "html", "--format", "json"])
        .assert()
        .success();
    let site = shop.join("docs/api");
    // The built version's model (from its frozen tree) and the working
    // tree's agree.
    for version in ["0.1.0-dev.1", "unreleased"] {
        let model: serde_json::Value =
            serde_json::from_str(&read(&site.join(format!("json/{version}/model.json")))).unwrap();
        let modules: Vec<&str> = model["modules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["path"].as_str().unwrap())
            .collect();
        assert_eq!(modules, ["api"], "{version}");
        assert_eq!(model["dependencies"][0]["package"], "std");
        let reached: Vec<&str> = model["dependencies"][0]["modules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["path"].as_str().unwrap())
            .collect();
        assert_eq!(reached, ["http"], "{version}");
    }
    let api = read(&site.join("html/0.1.0-dev.1/api.html"));
    assert!(
        api.contains("href=\"https://comlineproject.github.io/docs/std/http/#httpmethod\""),
        "{api}"
    );
    assert!(api.contains("<summary>Dependencies</summary>"), "{api}");
    assert!(!site.join("html/0.1.0-dev.1/std").exists());
    assert!(
        api.contains("<a class=\"brand\" href=\"index.html\">shop</a>"),
        "{api}"
    );
}

/// A `$BROWSER` that records what it was asked to open instead of opening it.
#[cfg(unix)]
fn recording_browser(dir: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let log = dir.join("opened.txt");
    let script = dir.join("browser.sh");
    fs::write(
        &script,
        format!("#!/bin/sh\necho \"$1\" >> '{}'\n", log.display()),
    )
    .unwrap();
    fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
    (script, log)
}

/// What the recording browser was asked to open; it runs detached, so wait for it.
#[cfg(unix)]
fn opened(log: &Path) -> String {
    for _ in 0..100 {
        if let Ok(text) = fs::read_to_string(log) {
            if text.ends_with('\n') {
                return text.trim().to_string();
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    panic!("nothing opened {}", log.display());
}

#[cfg(unix)]
#[test]
fn open_renders_a_missing_output_first_and_opens_it() {
    let temp = tempfile::tempdir().unwrap();
    let shop = temp.path().join("shop");
    package(&shop, "", API);
    fs::write(shop.join("comline.toml"), "[doc]\nformats = [\"html\"]\n").unwrap();
    let (browser, log) = recording_browser(temp.path());

    let index = shop.join("docs/api/html/index.html");
    comline_cmd()
        .current_dir(&shop)
        .env("BROWSER", &browser)
        .args(["doc", "open"])
        .assert()
        .success()
        .stderr(predicate::str::contains("Documented 1 version(s) of shop"))
        .stderr(predicate::str::contains(index.display().to_string()));

    assert!(index.exists());
    assert_eq!(opened(&log), index.display().to_string());
}

#[cfg(unix)]
#[test]
fn open_uses_existing_output_without_rendering_and_respects_out() {
    let temp = tempfile::tempdir().unwrap();
    let shop = temp.path().join("shop");
    package(&shop, "", API);
    let (browser, log) = recording_browser(temp.path());
    comline_cmd()
        .current_dir(&shop)
        .args(["doc", "--format", "html", "--out", "site"])
        .assert()
        .success();
    let index = shop.join("site/html/index.html");
    // Marks the output, so a re-render would be seen.
    fs::write(&index, "<!-- kept -->").unwrap();

    // Markdown is configured by default, but HTML is asked for by name.
    comline_cmd()
        .current_dir(&shop)
        .env("BROWSER", &browser)
        .args(["doc", "open", "html", "--out", "site"])
        .assert()
        .success()
        .stderr(predicate::str::contains("as they are"))
        .stderr(predicate::str::contains("comline doc --format html"))
        .stderr(predicate::str::contains("Documented").not());

    assert_eq!(read(&index), "<!-- kept -->");
    assert_eq!(opened(&log), index.display().to_string());
}

#[cfg(unix)]
#[test]
fn doc_open_flag_renders_then_opens_the_first_format() {
    let temp = tempfile::tempdir().unwrap();
    let shop = temp.path().join("shop");
    package(&shop, "", API);
    let (browser, log) = recording_browser(temp.path());
    comline_cmd()
        .current_dir(&shop)
        .env("BROWSER", &browser)
        .args(["doc", "--format", "html,markdown", "--open"])
        .assert()
        .success();
    assert_eq!(
        opened(&log),
        shop.join("docs/api/html/index.html").display().to_string()
    );
}

#[test]
fn open_names_the_file_and_succeeds_when_nothing_can_open_it() {
    let temp = tempfile::tempdir().unwrap();
    let shop = temp.path().join("shop");
    package(&shop, "", API);
    comline_cmd()
        .current_dir(&shop)
        .env("BROWSER", temp.path().join("no-such-browser"))
        .args(["doc", "open", "html"])
        .assert()
        .success()
        .stderr(predicate::str::contains("index.html"))
        .stderr(predicate::str::contains("$BROWSER"));
}

#[test]
fn open_rejects_a_format_it_has_no_index_for() {
    let temp = tempfile::tempdir().unwrap();
    let shop = temp.path().join("shop");
    package(&shop, "", API);
    comline_cmd()
        .current_dir(&shop)
        .args(["doc", "open", "pdf"])
        .assert()
        .failure()
        .stderr(predicate::str::contains(
            "only `html`, `markdown` and `json`",
        ));
}
