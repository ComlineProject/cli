//! `comline clean` — remove build artifacts.

use std::path::{Path, PathBuf};

use comline_core::package::build;
use comline_core::package::config::ir::frozen::FrozenUnit as ConfigUnit;
use miette::{IntoDiagnostic, Result, WrapErr};

use crate::commands::ensure_project;
use crate::ui;

pub fn run(work_dir: &Path, dry_run: bool) -> Result<()> {
    ensure_project(work_dir)?;

    let mut targets: Vec<PathBuf> = Vec::new();

    let cas = work_dir.join(".comline");
    if cas.exists() {
        targets.push(cas);
    }
    targets.extend(generated_files(work_dir));
    targets.sort();
    targets.dedup();

    if targets.is_empty() {
        ui::success("Nothing to clean");
        return Ok(());
    }

    for target in &targets {
        let shown = target.strip_prefix(work_dir).unwrap_or(target).display();
        if dry_run {
            ui::detail(format!("would remove {shown}"));
            continue;
        }
        remove(target).wrap_err_with(|| format!("failed to remove `{}`", target.display()))?;
        ui::detail(format!("removed {shown}"));
    }

    ui::success(if dry_run {
        "Dry run complete — nothing was deleted"
    } else {
        "Clean complete"
    });
    Ok(())
}

/// Best-effort list of files `generate` would have written: `<namespace>.<ext>`
/// next to `config.idp`, for every configured target. Returns an empty list if
/// the project does not currently compile.
fn generated_files(work_dir: &Path) -> Vec<PathBuf> {
    let Ok(context) = build::compile_package(work_dir) else {
        ui::note("skipping generated-file cleanup: project does not compile");
        return Vec::new();
    };
    let Some(config_units) = &context.config_frozen else {
        return Vec::new();
    };

    let mut files = Vec::new();
    for unit in config_units {
        let ConfigUnit::CodeGeneration(generation) = unit else {
            continue;
        };
        let Some((lang, version)) = generation.name.split_once('#') else {
            continue;
        };
        let Some((_, ext)) = comline_core::codelib_gen::find_generator(lang, version) else {
            continue;
        };
        for schema_ctx in &context.schema_contexts {
            let schema_ctx = schema_ctx.borrow();
            let candidate = work_dir.join(format!("{}.{}", schema_ctx.namespace_joined(), ext));
            if candidate.is_file() {
                files.push(candidate);
            }
        }
    }
    files
}

fn remove(path: &Path) -> Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path).into_diagnostic()
    } else {
        std::fs::remove_file(path).into_diagnostic()
    }
}
