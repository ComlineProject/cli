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

use std::collections::HashMap;
use std::path::Path;

use comline_codelib_gen::code_gen::{self, GenRequest, Mode, PackageMeta};
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

    // Crate / package name for a `lib`-mode manifest — the congregation name,
    // `::` flattened to `-` so it is a valid crate name.
    let package_name = context.config.name.value.replace("::", "-");

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
        let mode = match t.mode.as_str() {
            "code" => Mode::Code,
            "lib" => Mode::Lib,
            other => {
                return Err(miette!(
                    "target `{}`: mode `{other}` is not supported yet (`code`, `lib`)",
                    t.language
                ))
            }
        };

        let Some((generator, ext)) = code_gen::find_generator(&t.language, &t.lang_version) else {
            return Err(miette!(
                "no generator for `{}` (version `{}`)",
                t.language,
                t.lang_version
            ));
        };

        let versions = expand_versions(&t.versions, work_dir, &live_schemas)?;
        if mode == Mode::Lib && versions.len() > 1 {
            return Err(miette!(
                "target `{}`: `lib` mode does not support multiple package versions yet",
                t.language
            ));
        }
        if mode == Mode::Code && versions.len() > 1 && !t.layout.contains("{{package_version}}") {
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
            let request = GenRequest {
                mode,
                schemas: &gv.schemas,
                package: PackageMeta {
                    name: package_name.clone(),
                    // An unbuilt working tree has no version yet; `0.0.0` keeps
                    // the generated manifest valid.
                    version: if gv.package_version.is_empty() {
                        "0.0.0".to_string()
                    } else {
                        gv.package_version.clone()
                    },
                },
            };
            let files =
                generator(&request).map_err(|e| miette!("`{}` generator: {e}", t.language))?;

            match mode {
                // One file per schema, placed by `layout`.
                Mode::Code => {
                    let by_namespace: HashMap<String, &str> = files
                        .iter()
                        .map(|f| {
                            (
                                f.path.with_extension("").to_string_lossy().into_owned(),
                                f.contents.as_str(),
                            )
                        })
                        .collect();
                    for (namespace, _) in &gv.schemas {
                        let contents = by_namespace.get(namespace.as_str()).ok_or_else(|| {
                            miette!(
                                "`{}` generator produced no file for `{namespace}`",
                                t.language
                            )
                        })?;
                        let dest =
                            t.dest_for(namespace, ext, &spec_version, &gv.package_version)?;
                        write_file(&dest, contents)?;

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
                // A crate at `<out>/<language>/`; the generator owns the layout inside it.
                Mode::Lib => {
                    let root = t.out.join(&t.language);
                    for f in &files {
                        let dest = root.join(&f.path);
                        write_file(&dest, &f.contents)?;
                        let shown = dest.strip_prefix(work_dir).unwrap_or(&dest).display();
                        ui::detail(format!("     {} {} {shown}", f.path.display(), ui::arrow()));
                        written += 1;
                    }
                }
            }
        }
    }

    ui::success(format!("Generated {written} file(s)"));
    Ok(())
}

fn write_file(dest: &Path, contents: &str) -> Result<()> {
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent)
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to create `{}`", parent.display()))?;
    }
    std::fs::write(dest, contents)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to write `{}`", dest.display()))
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
