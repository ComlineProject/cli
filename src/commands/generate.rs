//! `comline generate` — validate, then write generated code for each target.
//!
//! Output location, layout, mode and version selection come from `comline.toml`
//! `[generate]` (`--out` / `--layout` / `--mode` override per run). The target
//! list comes from `[[generate.target]]` there, or — if none is listed — from
//! `code_generation.languages` in `config.idp`.
//!
//! `package_versions = "latest"` (the default) generates the working tree with
//! `compile_package` — no `.comline/` write, no version bump. `"all"` or an
//! explicit list reads committed versions from the CAS chain instead.

use std::path::Path;

use comline_core::package::build::{self, cas::ObjectStore};
use comline_core::package::config::ir::frozen::FrozenUnit as ConfigUnit;
use comline_core::schema::idl::constants::SCHEMA_EXTENSION;
use comline_core::schema::ir::frozen::unit::{schema_namespace_as_path, FrozenUnit};
use miette::{miette, IntoDiagnostic, Result, WrapErr};

use crate::commands::{core_err, ensure_project};
use crate::gen_config::{self, ComlineToml, DeclaredLang, Overrides, VersionSpec};
use crate::{history, ui, watch};

pub fn run(work_dir: &Path, overrides: &Overrides, watching: bool) -> Result<()> {
    ensure_project(work_dir)?;

    if watching {
        watch::run(work_dir, "generate", || generate_once(work_dir, overrides))
    } else {
        generate_once(work_dir, overrides)
    }
}

/// One package version to emit: its version string and the schemas it had.
struct GenVersion {
    package_version: String,
    schemas: Vec<(String, Vec<FrozenUnit>)>, // (namespace path, frozen units)
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

    // The working-tree schemas, used for `package_versions = "latest"`.
    let live_schemas: Vec<(String, Vec<FrozenUnit>)> = context
        .schema_contexts
        .iter()
        .filter_map(|sc| {
            let sc = sc.borrow();
            let units = sc.frozen_schema.borrow().clone()?;
            Some((sc.namespace.join("/"), units))
        })
        .collect();

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
            comline_codelib_gen::code_gen::find_generator(&t.language, &t.lang_version)
        else {
            return Err(miette!(
                "no generator for `{}` (version `{}`)",
                t.language,
                t.lang_version
            ));
        };

        let versions = expand_versions(&t.versions, work_dir, &live_schemas)?;
        if versions.len() > 1 && !t.layout.contains("{{package_version}}") {
            return Err(miette!(
                "target `{}` selects {} versions but its `layout` has no \
                 `{{{{package_version}}}}` — every version would write to the same paths",
                t.language,
                versions.len()
            ));
        }
        let multi = versions.len() > 1;

        let label = format!("language: {}, version: {}", t.language, t.lang_version);
        ui::detail(if ui::plain() {
            label
        } else {
            format!("⚙️  {label}")
        });

        for gv in &versions {
            for (namespace, units) in &gv.schemas {
                let dest = t.dest_for(namespace, ext, &spec_version, &gv.package_version)?;

                if let Some(parent) = dest.parent() {
                    std::fs::create_dir_all(parent)
                        .into_diagnostic()
                        .wrap_err_with(|| format!("failed to create `{}`", parent.display()))?;
                }
                std::fs::write(&dest, generator(units))
                    .into_diagnostic()
                    .wrap_err_with(|| format!("failed to write `{}`", dest.display()))?;

                let src_name = format!("{namespace}.{SCHEMA_EXTENSION}");
                let shown = dest.strip_prefix(work_dir).unwrap_or(&dest).display();
                let tag = if multi {
                    format!("[{}] ", gv.package_version)
                } else {
                    String::new()
                };
                ui::detail(format!("     {tag}{src_name} {} {shown}", ui::arrow()));
                written += 1;
            }
        }
    }

    ui::success(format!("Generated {written} file(s)"));
    Ok(())
}

/// Turn a [`VersionSpec`] into the concrete list of versions to emit.
///
/// `Latest` is the working tree (`live`), stamped with the last committed
/// version if there is one. `All` / `List` read the CAS chain and require the
/// project to have been built.
fn expand_versions(
    spec: &VersionSpec,
    work_dir: &Path,
    live: &[(String, Vec<FrozenUnit>)],
) -> Result<Vec<GenVersion>> {
    match spec {
        VersionSpec::Latest => {
            let package_version = history::load(work_dir)
                .ok()
                .and_then(|chain| chain.first().map(|e| e.commit.version.clone()))
                .unwrap_or_default();
            Ok(vec![GenVersion {
                package_version,
                schemas: live.to_vec(),
            }])
        }
        VersionSpec::All => {
            let chain = history::load(work_dir)?;
            let store = ObjectStore::new(work_dir);
            chain
                .iter()
                .map(|e| gen_version_from_commit(&store, &e.commit.version, &e.commit))
                .collect()
        }
        VersionSpec::List(specs) => {
            let chain = history::load(work_dir)?;
            let store = ObjectStore::new(work_dir);
            specs
                .iter()
                .map(|s| {
                    let commit = history::resolve(&chain, s)?;
                    gen_version_from_commit(&store, &commit.version, commit)
                })
                .collect()
        }
    }
}

fn gen_version_from_commit(
    store: &ObjectStore,
    version: &str,
    commit: &comline_core::package::build::cas::objects::Commit,
) -> Result<GenVersion> {
    let schemas = history::load_schemas(store, commit)?
        .into_iter()
        .map(|units| {
            let ns = schema_namespace_as_path(&units).unwrap_or_default();
            (ns, units)
        })
        .collect();
    Ok(GenVersion {
        package_version: version.to_owned(),
        schemas,
    })
}
