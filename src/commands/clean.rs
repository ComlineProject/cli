//! `comline clean` — remove generated code.
//!
//! Only the output of `comline generate`. The `.comline/` store (the version
//! history) is left alone — [`super::reset`] is the command that discards that.

use std::path::{Path, PathBuf};

use comline_core::package::build;
use comline_core::package::config::ir::frozen::FrozenUnit as ConfigUnit;
use miette::{IntoDiagnostic, Result, WrapErr};

use crate::commands::ensure_project;
use crate::gen_config::{self, ComlineToml, DeclaredLang, Overrides};
use crate::ui;

pub fn run(work_dir: &Path, dry_run: bool) -> Result<()> {
    ensure_project(work_dir)?;

    ui::step(format!("Cleaning{}", ui::at_path(work_dir)));

    let mut targets: Vec<PathBuf> = generated_files(work_dir);
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

/// Best-effort list of what `generate` would have written, resolved the same way
/// `generate` resolves it (`comline.toml` `[generate]`, else the congregation's
/// declared languages). For each target this is the whole output root when it is
/// a real sub-directory, otherwise the individual rendered files. Empty if the
/// project does not currently compile or `comline.toml` does not parse.
///
/// Shared with [`super::reset`], which removes these too.
pub(crate) fn generated_files(work_dir: &Path) -> Vec<PathBuf> {
    let cfg = match ComlineToml::load(work_dir) {
        Ok(cfg) => cfg,
        Err(e) => {
            ui::note(format!("skipping generated-file cleanup: {e}"));
            return Vec::new();
        }
    };
    let Ok(context) = build::compile_package(work_dir) else {
        ui::note("skipping generated-file cleanup: project does not compile");
        return Vec::new();
    };
    let Some(config_units) = &context.config_frozen else {
        return Vec::new();
    };

    let declared: Vec<DeclaredLang> = config_units
        .iter()
        .filter_map(|u| match u {
            ConfigUnit::CodeGeneration(g) => {
                g.name.split_once('#').map(|(lang, ver)| DeclaredLang {
                    language: lang.to_owned(),
                    lang_version: ver.to_owned(),
                })
            }
            _ => None,
        })
        .collect();
    let spec_version = config_units
        .iter()
        .find_map(|u| match u {
            ConfigUnit::SpecificationVersion(v) => Some(v.to_string()),
            _ => None,
        })
        .unwrap_or_default();

    let Ok(targets) = gen_config::resolve(&cfg, &declared, work_dir, &Overrides::default()) else {
        return Vec::new();
    };

    let mut files = Vec::new();
    for t in &targets {
        // A dedicated output directory: remove it wholesale (stale files too).
        if t.out != work_dir && t.out.is_dir() {
            files.push(t.out.clone());
            continue;
        }
        // Output lands among the sources — only touch the exact files. This
        // covers the single-version (`latest`) case; a multi-version tree lives
        // under a dedicated `out` dir and is removed by the branch above.
        let Some((_, ext)) =
            comline_core::codelib_gen::find_generator(&t.language, &t.lang_version)
        else {
            continue;
        };
        for schema_ctx in &context.schema_contexts {
            let schema_ctx = schema_ctx.borrow();
            let namespace = schema_ctx.namespace.join("/");
            let Ok(candidate) = t.dest_for(&namespace, ext, &spec_version, "") else {
                continue;
            };
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
