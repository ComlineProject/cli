//! `comline targets` — print every `language#version` this build of
//! `comline` can generate, to stdout: each generator's floor, then the
//! versions it handles and has verified. With `--features`, what each of them
//! supports, and from which version.
//!
//! Reflects how this binary was compiled (which `gen-<lang>` Cargo features
//! were on), not any particular project — ignores `--path` entirely, same as
//! `completions`.

use std::collections::BTreeMap;
use std::io::Write as _;

use comline_codegen::features::{self, Column, Feature};
use comline_codegen::versions::Versions;
use miette::{IntoDiagnostic, Result};

use crate::commands::generate::generator_registry;

pub fn run(with_features: bool, json: bool) -> Result<()> {
    let registry = generator_registry();
    let mut out = std::io::stdout().lock();

    if json {
        let columns: Vec<Column> = registry
            .targets()
            .into_iter()
            .filter_map(|t| {
                let resolved = registry.resolve(t.name, t.version).ok()?;
                Some(Column {
                    generator: t.name,
                    versions: t.versions,
                    support: resolved.generator.support,
                })
            })
            .collect();
        let report = features::json(&columns, &BTreeMap::new());
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&report).into_diagnostic()?
        )
        .into_diagnostic()?;
        return Ok(());
    }

    for t in registry.targets() {
        // `name#floor` first: the token `config.idp` takes, so anything
        // reading the first word keeps working. Then the range.
        writeln!(out, "{}#{}  {}", t.name, t.version, range(&t.versions)).into_diagnostic()?;
        let Some(resolved) = registry
            .resolve(t.name, t.version)
            .ok()
            .filter(|_| with_features)
        else {
            continue;
        };
        for &feature in Feature::ALL {
            let s = features::support_for(resolved.generator.support, feature);
            let mut line = format!("  {:<11}  {}", s.state.id(), feature.id());
            if let Some(since) = s.since {
                line.push_str(&format!(
                    " from {} (below it, unsupported: {})",
                    since.version, since.why
                ));
            }
            if let Some(note) = s.note {
                line.push_str(&format!(" - {note}"));
            }
            if let Some(tracking) = s.tracking {
                line.push_str(&format!(" ({tracking})"));
            }
            writeln!(out, "{line}").into_diagnostic()?;
        }
    }
    Ok(())
}

/// The versions a generator handles, as a reader wants them: from its floor
/// on, where its output changes, and how far it's verified.
fn range(versions: &Versions) -> String {
    let mut s = format!("{} or later", versions.floor);
    if !versions.changes.is_empty() {
        s.push_str(&format!(", changes at {}", versions.changes.join(", ")));
    }
    s.push_str(&format!("; verified up to {}", versions.verified_up_to));
    if versions.forward_compatible {
        s.push_str(", and newer toolchains build its output");
    }
    format!("({s})")
}
