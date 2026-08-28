//! `comline generate` — build, then run each configured code generator.

use std::path::Path;

use comline_core::package::build;
use comline_core::package::config::ir::frozen::FrozenUnit as ConfigUnit;
use miette::{IntoDiagnostic, Result, WrapErr};

use crate::commands::{core_err, ensure_project};
use crate::ui;
use crate::watch;

pub fn run(work_dir: &Path, target: Option<&str>, watching: bool) -> Result<()> {
    ensure_project(work_dir)?;

    if watching {
        watch::run(work_dir, "generate", || generate_once(work_dir, target))
    } else {
        generate_once(work_dir, target)
    }
}

fn generate_once(work_dir: &Path, target: Option<&str>) -> Result<()> {
    ui::step("Generating code");
    let spinner = ui::spinner("compiling schemas");
    let built = build::build(work_dir);
    ui::finish_spinner(spinner);
    let built = built.map_err(|e| core_err("code generation failed", e))?;

    let Some(config_units) = &built.context.config_frozen else {
        ui::warn("no `code_generation` targets configured in config.idp");
        return Ok(());
    };

    let mut matched_targets = 0usize;
    let mut written = 0usize;

    for unit in config_units {
        let ConfigUnit::CodeGeneration(generation) = unit else {
            continue;
        };
        let Some((lang, version)) = generation.name.split_once('#') else {
            ui::warn(format!(
                "skipping invalid language specifier `{}`",
                generation.name
            ));
            continue;
        };
        if let Some(wanted) = target {
            if !wanted.eq_ignore_ascii_case(lang) {
                continue;
            }
        }
        matched_targets += 1;

        let Some((generator, ext)) = comline_core::codelib_gen::find_generator(lang, version)
        else {
            ui::warn(format!("no generator for `{lang}` (version `{version}`)"));
            continue;
        };

        ui::detail(format!("target {lang} (v{version})"));
        for schema_ctx in &built.context.schema_contexts {
            let schema_ctx = schema_ctx.borrow();
            let Some(frozen_units) = schema_ctx.frozen_schema.borrow().clone() else {
                continue;
            };
            let output = generator(&frozen_units);
            let file_name = format!("{}.{}", schema_ctx.namespace_joined(), ext);
            std::fs::write(work_dir.join(&file_name), output)
                .into_diagnostic()
                .wrap_err_with(|| format!("failed to write `{file_name}`"))?;
            ui::detail(format!("  wrote {file_name}"));
            written += 1;
        }
    }

    if let Some(wanted) = target {
        if matched_targets == 0 {
            return Err(miette::miette!(
                "no configured `code_generation` target matches `{wanted}`"
            ));
        }
    }

    ui::success(format!("Generated {written} file(s)"));
    Ok(())
}
