//! `comline targets` — print every `language#version` this build of
//! `comline` can generate, to stdout; with `--features`, what each of them
//! supports.
//!
//! Reflects how this binary was compiled (which `gen-<lang>` Cargo features
//! were on), not any particular project — ignores `--path` entirely, same as
//! `completions`.

use std::collections::BTreeMap;
use std::io::Write as _;

use comline_codegen::features::{self, Column, Feature};
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
                let support = registry.support(t.name, t.version)?;
                Some(Column {
                    generator: t.name,
                    version: t.version,
                    support,
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
        writeln!(out, "{}#{}", t.name, t.version).into_diagnostic()?;
        let Some(support) = registry
            .support(t.name, t.version)
            .filter(|_| with_features)
        else {
            continue;
        };
        for &feature in Feature::ALL {
            let s = features::support_for(support, feature);
            let mut line = format!("  {:<11}  {}", s.state.id(), feature.id());
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
