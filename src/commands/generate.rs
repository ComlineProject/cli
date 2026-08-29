//! `comline generate` — validate, then write generated code for each target.
//!
//! Output location, layout and mode come from `comline.toml` `[generate]`
//! (overridable per run with `--out` / `--layout` / `--mode`). The target list
//! comes from `[[generate.target]]` there, or — if none is listed — from
//! `code_generation.languages` in `config.idp`.
//!
//! Unlike `build`, this never freezes a version: it uses `compile_package`, so
//! no `.comline/` write and no version bump.

use std::path::Path;

use comline_core::package::build;
use comline_core::package::config::ir::frozen::FrozenUnit as ConfigUnit;
use comline_core::schema::idl::constants::SCHEMA_EXTENSION;
use miette::{miette, IntoDiagnostic, Result, WrapErr};

use crate::commands::{core_err, ensure_project};
use crate::gen_config::{self, ComlineToml, DeclaredLang, Overrides};
use crate::ui;
use crate::watch;

pub fn run(work_dir: &Path, overrides: &Overrides, watching: bool) -> Result<()> {
    ensure_project(work_dir)?;

    if watching {
        watch::run(work_dir, "generate", || generate_once(work_dir, overrides))
    } else {
        generate_once(work_dir, overrides)
    }
}

fn generate_once(work_dir: &Path, overrides: &Overrides) -> Result<()> {
    ui::step(format!("Generating code{}", ui::at_path(work_dir)));

    let cfg = ComlineToml::load(work_dir)?;

    let spinner = ui::spinner("validating schemas");
    let context = build::compile_package(work_dir);
    ui::finish_spinner(spinner);
    let context = context.map_err(|e| core_err("code generation failed", e))?;

    let Some(config_units) = &context.config_frozen else {
        ui::warn("no package config was produced");
        return Ok(());
    };

    // Languages the congregation declared: `rust#1.70.0` -> ("rust", "1.70.0").
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

    let targets = gen_config::resolve(&cfg, &declared, work_dir, overrides)?;

    let mut written = 0usize;
    for t in &targets {
        if t.mode != "code" {
            return Err(miette!(
                "target `{}`: mode `{}` is not supported yet (only `code`)",
                t.language,
                t.mode
            ));
        }

        let Some((generator, ext)) =
            comline_core::codelib_gen::find_generator(&t.language, &t.lang_version)
        else {
            return Err(miette!(
                "no generator for `{}` (version `{}`)",
                t.language,
                t.lang_version
            ));
        };

        let label = format!("language: {}, version: {}", t.language, t.lang_version);
        ui::detail(if ui::plain() {
            label
        } else {
            format!("⚙️  {label}")
        });

        for schema_ctx in &context.schema_contexts {
            let schema_ctx = schema_ctx.borrow();
            let Some(frozen_units) = schema_ctx.frozen_schema.borrow().clone() else {
                continue;
            };
            let namespace = schema_ctx.namespace.join("/");
            let dest = t.dest_for(&namespace, ext, &spec_version)?;

            if let Some(parent) = dest.parent() {
                std::fs::create_dir_all(parent)
                    .into_diagnostic()
                    .wrap_err_with(|| format!("failed to create `{}`", parent.display()))?;
            }
            let code = generator(&frozen_units);
            std::fs::write(&dest, code)
                .into_diagnostic()
                .wrap_err_with(|| format!("failed to write `{}`", dest.display()))?;

            let src_name = format!("{namespace}.{SCHEMA_EXTENSION}");
            let shown = dest.strip_prefix(work_dir).unwrap_or(&dest).display();
            ui::detail(format!("     {src_name} {} {shown}", ui::arrow()));
            written += 1;
        }
    }

    ui::success(format!("Generated {written} file(s)"));
    Ok(())
}
