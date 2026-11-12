//! `comline deps list` — the dependencies `config.idp` declares, with where
//! each comes from, what it's pinned to, and what's accepted of it.
//!
//! Read-only, and it only reads `config.idp`: nothing is resolved, fetched or
//! written (no `.comline/`), so what it says is what the file says, not what
//! the dependency holds now. `--json` prints the same as JSON.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::Path;

use comline_core::package::config::dependency::{DependencyConfig, DependencySource};
use comline_core::schema::ir::frozen::unit::Guarantee;
use comline_core::schema::ir::unverified;
use miette::{IntoDiagnostic, Result, WrapErr};
use serde_json::json;

use crate::commands::add::declared_dependencies;
use crate::commands::ensure_project;
use crate::ui;

/// Characters of a commit shown in the text report.
const SHORT_COMMIT: usize = 7;
/// Characters of a hash shown in the text report, after `blake3:`.
const SHORT_HASH: usize = 12;

pub fn run(work_dir: &Path, json: bool) -> Result<()> {
    ensure_project(work_dir)?;
    let manifest = std::fs::read_to_string(work_dir.join("config.idp"))
        .into_diagnostic()
        .wrap_err("failed to read config.idp")?;
    let declared = declared_dependencies(&manifest)?;

    let mut out = anstream::stdout().lock();
    if json {
        let report = json!({ "dependencies": declared.iter().map(as_json).collect::<Vec<_>>() });
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&report).into_diagnostic()?
        )
        .into_diagnostic()?;
        return Ok(());
    }

    if declared.is_empty() {
        ui::note("No dependencies declared");
        return Ok(());
    }
    write!(out, "{}", render(&declared)).into_diagnostic()?;
    Ok(())
}

/// One line per dependency, in columns: name, source, pin, then what's
/// accepted if anything is.
pub fn render(declared: &[DependencyConfig]) -> String {
    let rows: Vec<[String; 4]> = declared
        .iter()
        .map(|d| {
            [
                d.name.clone(),
                source_text(&d.source),
                d.declared_hash().map_or("unpinned".to_string(), short_hash),
                accepted_text(&d.accept_unverified),
            ]
        })
        .collect();
    let width = |column: usize| {
        rows.iter()
            .map(|r| r[column].chars().count())
            .max()
            .unwrap_or(0)
    };
    let (name, source, pin) = (width(0), width(1), width(2));

    let mut out = String::new();
    for [n, s, p, a] in &rows {
        let line = format!("{n:<name$}  {s:<source$}  {p:<pin$}  {a}");
        out += line.trim_end();
        out += "\n";
    }
    out
}

fn source_text(source: &DependencySource) -> String {
    match source {
        DependencySource::Path { path, .. } => format!("path {}", slashed(path)),
        DependencySource::Git {
            uri,
            commit,
            version,
            ..
        } => format!(
            "git {uri}@{} ({version})",
            commit.chars().take(SHORT_COMMIT).collect::<String>()
        ),
        DependencySource::Registry { uri, version, .. } => format!("registry {uri} ({version})"),
    }
}

fn short_hash(hash: &str) -> String {
    match hash.split_once(':') {
        Some((algorithm, digest)) => format!(
            "{algorithm}:{}",
            digest.chars().take(SHORT_HASH).collect::<String>()
        ),
        None => hash.chars().take(SHORT_HASH).collect(),
    }
}

/// `accepts Billing (schema agreement and encoding agreement)`, or nothing.
fn accepted_text(accepted: &BTreeMap<String, Vec<Guarantee>>) -> String {
    if accepted.is_empty() {
        return String::new();
    }
    let protocols: Vec<String> = accepted
        .iter()
        .map(|(protocol, set)| match set.as_slice() {
            [] => format!("{protocol} (nothing)"),
            set => format!("{protocol} ({})", unverified::describe(set)),
        })
        .collect();
    format!("accepts {}", protocols.join(", "))
}

/// The entry as `config.idp` writes it, in full.
fn as_json(dependency: &DependencyConfig) -> serde_json::Value {
    let source = match &dependency.source {
        DependencySource::Path { path, .. } => json!({ "kind": "path", "path": slashed(path) }),
        DependencySource::Git {
            uri,
            commit,
            version,
            ..
        } => json!({ "kind": "git", "uri": uri, "commit": commit, "version": version }),
        DependencySource::Registry { uri, version, .. } => {
            json!({ "kind": "registry", "uri": uri, "version": version })
        }
    };
    let accepted: BTreeMap<&str, Vec<&str>> = dependency
        .accept_unverified
        .iter()
        .map(|(protocol, set)| (protocol.as_str(), set.iter().map(|g| g.name()).collect()))
        .collect();
    json!({
        "name": dependency.name,
        "source": source,
        "hash": dependency.declared_hash(),
        "accept_unverified": accepted,
    })
}

fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
