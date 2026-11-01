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
        .stderr(predicate::str::contains("Documented shop"));

    let out = shop.join("docs/api");
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
    let index = read(&shop.join("v1/index.md"));
    assert!(
        index.starts_with("# shop 0.1.0-dev.1\n\nA shop.\n"),
        "{index}"
    );
    let api = read(&shop.join("v1/api.md"));
    assert!(api.starts_with("# `api`\n\nThe shop's API.\n"), "{api}");
    assert!(!api.contains("note"));

    comline_cmd()
        .current_dir(&shop)
        .args(["doc", "--changelog-from", "0.1.0-dev.1", "--out", "now"])
        .assert()
        .success();
    let changelog = read(&shop.join("now/changelog.md"));
    assert!(
        changelog.contains("## 0.1.0-dev.1 to working tree"),
        "{changelog}"
    );
    assert!(changelog.contains("field name: `note`"), "{changelog}");
}

#[test]
fn json_writes_the_model_and_doc_settings_come_from_comline_toml() {
    let temp = tempfile::tempdir().unwrap();
    let shop = temp.path().join("shop");
    package(&shop, "", API);
    fs::write(
        shop.join("comline.toml"),
        "[doc]\nout = \"site\"\nformat = \"json\"\n",
    )
    .unwrap();

    comline_cmd()
        .current_dir(&shop)
        .arg("doc")
        .assert()
        .success();
    let model: serde_json::Value =
        serde_json::from_str(&read(&shop.join("site/model.json"))).unwrap();
    assert_eq!(model["model_version"], 1);
    assert_eq!(model["package"]["name"], "shop");
    assert_eq!(model["modules"][0]["docs"], "The shop's API.");
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
fn html_writes_a_self_contained_site() {
    let temp = tempfile::tempdir().unwrap();
    let shop = temp.path().join("shop");
    package(&shop, "//! A shop.\n\n", API);
    comline_cmd()
        .current_dir(&shop)
        .arg("build")
        .assert()
        .success();

    comline_cmd()
        .current_dir(&shop)
        .args([
            "doc",
            "--format",
            "html",
            "--out",
            "site",
            "--changelog-from",
            "0.1.0-dev.1",
        ])
        .assert()
        .success();

    let out = shop.join("site");
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
    assert!(read(&out.join("search.js")).contains("\"u\":\"api.html#order\""));
    assert!(out.join("style.css").exists());
    assert!(out.join("changelog.html").exists());
    assert!(!out.join("nav.yml").exists());
}

#[test]
fn html_is_a_doc_format_in_comline_toml() {
    let temp = tempfile::tempdir().unwrap();
    let shop = temp.path().join("shop");
    package(&shop, "", API);
    fs::write(
        shop.join("comline.toml"),
        "[doc]\nout = \"site\"\nformat = \"html\"\n",
    )
    .unwrap();
    comline_cmd()
        .current_dir(&shop)
        .arg("doc")
        .assert()
        .success();
    assert!(shop.join("site/index.html").exists());
    assert!(shop.join("site/api.html").exists());
}
