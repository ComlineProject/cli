//! `comline targets` — print every `language#version` this build of
//! `comline` can generate, to stdout.
//!
//! Reflects how this binary was compiled (which `gen-<lang>` Cargo features
//! were on), not any particular project — ignores `--path` entirely, same as
//! `completions`.

use std::io::Write as _;

use miette::{IntoDiagnostic, Result};

use crate::commands::generate::generator_registry;

pub fn run() -> Result<()> {
    let registry = generator_registry();
    let mut out = std::io::stdout().lock();
    for t in registry.targets() {
        writeln!(out, "{}#{}", t.name, t.version).into_diagnostic()?;
    }
    Ok(())
}
