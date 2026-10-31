//! `comline inspect`

use std::fs;
use std::path::Path;

use predicates::prelude::*;

use crate::util::*;

fn package(dir: &Path, config: &str, schema: &str) {
    fs::create_dir_all(dir.join("src")).unwrap();
    let name = dir.file_name().unwrap().to_str().unwrap();
    fs::write(
        dir.join("config.idp"),
        format!("congregation {name}\nspecification_version = 1\n{config}"),
    )
    .unwrap();
    fs::write(dir.join("src/api.ids"), schema).unwrap();
}

const ALLOW: &str = "settings = { annotations = { unverified = { allowed = true } framing = { allowed = true } } }\n";

fn shop(temp: &Path) -> std::path::PathBuf {
    let shop = temp.join("shop");
    package(
        &shop,
        &format!("wire = {{ framing = \"datagram@1\" }}\n{ALLOW}"),
        "protocol Orders {\n    function place(id: u64) -> bool;\n}\n\n\
         @framing = { name = \"jsonrpc\", method_names = \"qualified\" }\n\
         @handshake = \"none\"\n\
         @unverified = \"the 2009 gateway can't run a handshake\"\n\
         protocol Legacy {\n    function ping();\n}\n",
    );
    shop
}

#[test]
fn shows_each_choice_its_source_and_every_guarantee() {
    let temp = tempfile::tempdir().unwrap();
    let shop = shop(temp.path());

    let out = comline_cmd()
        .current_dir(&shop)
        .args(["--plain", "inspect"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let out = String::from_utf8(out).unwrap();
    let expected = "\
protocol api::Orders (shop)
  format     packed@1                 core's default
  framing    datagram                 from config.idp `wire.framing`
  handshake  comline@1                core's default

  schema agreement       held
  encoding agreement     held
  encoding conformance   held
  version honesty        held

protocol api::Legacy (shop)
  format     json@1                   JSON-RPC's format
  framing    jsonrpc (method_names = qualified) from @framing
  handshake  none                     from @handshake
  unverified \"the 2009 gateway can't run a handshake\"

  schema agreement       given up
      because handshake `none`
      acceptance: up to each consumer's config.idp
  encoding agreement     given up
      because handshake `none`
      acceptance: up to each consumer's config.idp
  encoding conformance   held
  version honesty        held
";
    assert_eq!(out, expected);
    assert!(out.lines().all(|l| l.chars().count() <= 80), "{out}");
    assert!(!shop.join(".comline").exists(), "inspect must not create the CAS store");
}

#[test]
fn json_carries_the_same() {
    let temp = tempfile::tempdir().unwrap();
    let shop = shop(temp.path());

    let out = comline_cmd()
        .current_dir(&shop)
        .args(["inspect", "--json"])
        .assert()
        .success()
        .get_output()
        .stdout
        .clone();
    let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
    let legacy = &json["protocols"][1];
    assert_eq!(legacy["name"], "Legacy");
    assert_eq!(legacy["framing"]["source"], "annotation");
    assert_eq!(legacy["framing"]["params"]["method_names"], "qualified");
    assert_eq!(legacy["format"]["source"], "framing");
    assert_eq!(json["protocols"][0]["framing"]["source"], "package_default");
    assert_eq!(json["protocols"][0]["format"]["source"], "core_default");
    let row = &legacy["guarantees"][0];
    assert_eq!(row["guarantee"], "schema_agreement");
    assert_eq!(row["held"], false);
    assert_eq!(row["causes"], serde_json::json!(["handshake `none`"]));
    assert_eq!(row["reason"], "the 2009 gateway can't run a handshake");
    assert_eq!(row["acceptance"], serde_json::json!({ "state": "unknown" }));
    assert_eq!(legacy["guarantees"][3]["acceptance"], serde_json::json!({ "state": "not_needed" }));
}

#[test]
fn a_dependencys_protocol_shows_who_accepted_it() {
    let temp = tempfile::tempdir().unwrap();
    package(
        &temp.path().join("billing"),
        &format!("handshake = \"none\"\n{ALLOW}"),
        "@unverified = \"checked out of band\"\nprotocol Billing {\n    function charge();\n}\n",
    );
    let app = temp.path().join("app");
    package(
        &app,
        "dependencies = {\n    billing = {\n        path = \"../billing\"\n        \
         accept_unverified = { Billing = [\"schema_agreement\", \"encoding_agreement\"] }\n    }\n}\n",
        "struct A {\n    x: u32\n}\n",
    );

    comline_cmd()
        .current_dir(&app)
        .args(["--plain", "inspect"])
        .assert()
        .success()
        .stdout(predicate::str::contains("protocol billing::api::Billing (dependency billing)"))
        .stdout(predicate::str::contains("handshake  none                     from config.idp `handshake`"))
        .stdout(predicate::str::contains("      because handshake `none`\n      accepted by app\n"));
}

#[test]
fn a_project_without_protocols_says_so() {
    let temp = tempfile::tempdir().unwrap();
    let app = temp.path().join("app");
    package(&app, "", "struct A {\n    x: u32\n}\n");

    comline_cmd()
        .current_dir(&app)
        .args(["--plain", "inspect"])
        .assert()
        .success()
        .stdout("")
        .stderr(predicate::str::contains("No protocols to inspect"));
}

#[test]
fn reports_a_broken_schema() {
    let temp = tempfile::tempdir().unwrap();
    let project = fixture_project(temp.path());
    fs::write(project.join("src/api.ids"), "struct Broken {").unwrap();

    comline_cmd()
        .current_dir(&project)
        .arg("inspect")
        .assert()
        .failure()
        .code(1)
        .stderr(predicate::str::contains("validation failed"));
}
