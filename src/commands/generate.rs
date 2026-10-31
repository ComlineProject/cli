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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use comline_codegen::protocol::{Description, TargetLanguage};
use comline_codegen::versions::LangVersion;
use comline_codegen::{
    connections, features, GenerateRequest, Mode, PackageMeta, Registry, Resolved, Unresolved,
};
use comline_core::package::build::{self, cas::ObjectStore};
use comline_core::package::config::ir::frozen::{self, FrozenUnit as ConfigUnit};
use comline_core::schema::idl::constants::SCHEMA_EXTENSION;
use comline_core::schema::ir::frozen::unit::{schema_namespace_as_path, FrozenUnit};
use miette::{miette, IntoDiagnostic, Result, WrapErr};

use crate::commands::{core_err, ensure_project};
use crate::external::{self, External};
use crate::gen_config::Destination;
use crate::lang_targets::{self, TargetError};
use crate::{history, ui, watch};
use comline_codegen::config::{
    self as gen_config, ComlineToml, DeclaredLang, Overrides, VersionSpec,
};

/// The generator registry, composed from the per-language `comline-codegen-*`
/// crates this CLI was built with — one `register()` per crate, each behind its
/// `gen-<lang>` cargo feature (both on by default). A build without a feature
/// never compiles that generator crate or its dependency tree.
pub(crate) fn generator_registry() -> Registry {
    #[allow(unused_mut)]
    let mut registry = Registry::new();
    #[cfg(feature = "gen-rust")]
    comline_codegen_rust::register(&mut registry);
    #[cfg(feature = "gen-typescript")]
    comline_codegen_typescript::register(&mut registry);
    registry
}

pub fn run(
    work_dir: &Path,
    overrides: &Overrides,
    dry_run: bool,
    list: bool,
    watching: bool,
) -> Result<()> {
    ensure_project(work_dir)?;

    if watching {
        watch::run(work_dir, "generate", || {
            generate_once(work_dir, overrides, dry_run, list)
        })
    } else {
        generate_once(work_dir, overrides, dry_run, list)
    }
}

/// One package version to emit: its version string and the schemas it had.
struct GenVersion {
    package_version: String,
    schemas: Vec<(String, Vec<FrozenUnit>)>, // (namespace path, frozen units)
    /// The version's own `wire.format`: a built version keeps the format it
    /// was built with.
    wire_format: String,
}

fn generate_once(work_dir: &Path, overrides: &Overrides, dry_run: bool, list: bool) -> Result<()> {
    ui::step(format!(
        "Generating code{}{}",
        ui::at_path(work_dir),
        if dry_run {
            " - dry run, nothing written"
        } else {
            ""
        },
    ));

    let mut cfg = ComlineToml::load(work_dir)?;

    let spinner = ui::spinner("validating schemas");
    let context = build::compile_package(work_dir);
    ui::finish_spinner(spinner);
    let context = context.map_err(|e| core_err("code generation failed", e))?;
    for warning in context.rendered_warnings() {
        ui::warn(warning);
    }

    let Some(config_units) = &context.config_frozen else {
        ui::warn("no package config was produced");
        return Ok(());
    };

    // Languages the congregation declared: `rust#1.85.0` -> ("rust", "1.85.0").
    let declared: Vec<DeclaredLang> = lang_targets::declared(config_units);

    let spec_version = config_units
        .iter()
        .find_map(|u| match u {
            ConfigUnit::SpecificationVersion(v) => Some(v.to_string()),
            _ => None,
        })
        .unwrap_or_default();

    // The encodings the package and its dependencies declare, each with its
    // suggested mappings: what a target's `encodings` in comline.toml
    // override, beside the official ones every package sees.
    let declared_encodings: Vec<ConfigUnit> = config_units
        .iter()
        .chain(
            context
                .dependency_configs
                .iter()
                .flat_map(|(_, units)| units),
        )
        .filter(|u| matches!(u, ConfigUnit::Encoding { .. }))
        .cloned()
        .collect();

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

    // External generators, described once each. A target that names one
    // and no `language` takes the one language it offers.
    let mut externals = Externals::default();
    for t in &mut cfg.generate.targets {
        if let (Some(generator), true) = (&t.generator, t.language.is_empty()) {
            let timeout = t
                .timeout
                .map(Duration::from_secs)
                .unwrap_or(external::DEFAULT_TIMEOUT);
            let (generator, description) = externals.get(&work_dir.join(generator), timeout)?;
            t.language = external::only_language(&generator.name, description)?;
        }
    }

    let targets = gen_config::resolve(&cfg, &declared, work_dir, overrides)?;

    let registry = generator_registry();

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

        if mode == Mode::Lib && t.layout != gen_config::DEFAULT_LAYOUT {
            let lands_at = if t.flatten {
                "<out>/".to_string()
            } else {
                format!("<out>/{}/", t.language)
            };
            ui::warn(format!(
                "target `{}`: `layout` (`{}`) has no effect in `lib` mode — the \
                 crate lands at `{lands_at}`",
                t.language, t.layout,
            ));
        }
        if mode == Mode::Code && t.flatten {
            ui::warn(format!(
                "target `{}`: `flatten` has no effect in `code` mode — it only \
                 changes where a `lib`-mode crate lands",
                t.language
            ));
        }

        let (source, ext) = match &t.generator {
            Some(path) => {
                let (generator, description) = externals.get(path, t.timeout)?;
                let info =
                    external::select(&generator.name, description, &t.language, &t.lang_version)?;
                if let Some((dependency, written, minimum)) =
                    lang_targets::dependency_minimum(&context, &t.language)
                {
                    if LangVersion::parse(&t.lang_version).is_some_and(|v| v < minimum) {
                        return Err(miette!(
                            "target `{}`: `{}#{}` is older than dependency `{dependency}` allows: \
                             it declares `{}#{written}`",
                            t.language,
                            t.language,
                            t.lang_version,
                            t.language,
                        ));
                    }
                }
                let ext = info.ext.clone();
                (Source::External(generator), ext)
            }
            None => {
                // Reached for every language the congregation declares now, not only
                // for one named by a `[[generate.target]]`, so it says what to do
                // about it: `only` is how a consumer whose binary can't produce a
                // language says so on purpose.
                let resolved: Resolved = match lang_targets::resolve(
                    &registry,
                    &context,
                    &t.language,
                    &t.lang_version,
                ) {
                    Ok(resolved) => resolved,
                    Err(TargetError::Unresolved(Unresolved::UnknownLanguage { .. })) => {
                        return Err(miette!(
                            "no generator for `{}` (version `{}`)\n  \
                     help: this build of comline has no {} generator. Install one that \
                     does, or narrow the set in {}:\n      \
                     [generate]\n      only = [...]",
                            t.language,
                            t.lang_version,
                            t.language,
                            gen_config::FILE_NAME,
                        ))
                    }
                    Err(e) => return Err(miette!("target `{}`: {e}", t.language)),
                };
                match lang_targets::unverified(&context, &t.language, &resolved) {
                    Ok(Some(warning)) => ui::warn(warning),
                    Ok(None) => {}
                    Err(error) => return Err(miette!("target `{}`: {error}", t.language)),
                }
                let ext = resolved.generator.ext.to_string();
                (Source::Builtin(resolved), ext)
            }
        };

        let versions = expand_versions(&t.versions, work_dir, &live_schemas, config_units)?;
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

        let label = format!(
            "language: {}, version: {}, mode: {}",
            t.language, t.lang_version, t.mode,
        );
        ui::detail(if ui::plain() {
            label
        } else {
            format!("⚙️  {label}")
        });

        for gv in &versions {
            let mut request = GenerateRequest {
                mode,
                schemas: gv.schemas.clone(),
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
                target: TargetLanguage {
                    language: t.language.clone(),
                    version: t.lang_version.clone(),
                },
                wire_format: gv.wire_format.clone(),
                external_std: t.external_std,
                async_api: t.async_api,
                validate_on_decode: t.validate_on_decode,
                // Filled just below.
                encodings: Default::default(),
            };
            // The type each encoding the package uses maps to in this
            // language: comline.toml's mapping, else the declaring package's
            // suggestion. One left with neither is refused here, by name.
            if let Err(e) = comline_codegen::encodings::fill(
                &mut request,
                comline_core::encodings::official::units(),
                &declared_encodings,
                &t.encodings,
            ) {
                // A built-in generator's support table is what says why it
                // can't take this package (TypeScript has no MessagePack
                // codec, and a tracking link for it); an encoding it has no
                // mapping for is that same fact, so the table's refusal comes
                // first, worded as the generator's own.
                if let Source::Builtin(resolved) = &source {
                    let options = features::Options::of(&request);
                    if let Some(refusal) =
                        lang_targets::unsupported(&t.language, resolved, &request.schemas, &options)
                    {
                        return Err(miette!("`{}` generator: {refusal}", t.language));
                    }
                }
                return Err(miette!("target `{}`: {e}", t.language));
            }
            // Deliberately not `core_err`: a generator's errors are about
            // schema/package *structure* (e.g. a namespace that's both a
            // schema and a parent of deeper schemas), not source *text* -
            // they carry no span to render a snippet against, so there's
            // nothing for the multi-line branch there to do.
            if let Source::Builtin(resolved) = &source {
                warn_ignored(&t.language, &request, resolved);
            }
            warn_async_unused(&t.language, &request);
            warn_no_package_connection(&t.language, &request);
            // Both kinds answer the same protocol request, and either
            // response is checked whole - every path, the total size -
            // before anything below writes a file.
            let files = match &source {
                Source::Builtin(_) => external::builtin(&registry, &t.language, request)?,
                Source::External(generator) => generator.generate(request)?,
            };

            // Every destination, whether or not it gets written: the tree
            // is drawn from these once the loop is done. A dry run differs
            // only in skipping `write_file` - everything above has already
            // happened for real (the generators are pure, and `write_file`
            // is the only side effect in this command), so the tree reports
            // what *would* be written rather than guessing at it.
            let mut destinations: Vec<PathBuf> = Vec::new();

            match mode {
                // One file per schema, placed by `layout` - whichever schemas
                // the generator actually produced a file for (it may
                // legitimately omit some, e.g. a reference to an external
                // dependency it doesn't inline).
                Mode::Code => {
                    for f in &files {
                        let namespace = f
                            .path_buf()
                            .with_extension("")
                            .to_string_lossy()
                            .into_owned();
                        let dest =
                            t.dest_for(&namespace, &ext, &spec_version, &gv.package_version)?;
                        if !dry_run {
                            write_file(&dest, &f.contents)?;
                        }
                        if list {
                            // The schema each file came from, which the tree
                            // can't show - the one thing `--list` has that
                            // the default doesn't.
                            let src_name = format!("{namespace}.{SCHEMA_EXTENSION}");
                            let shown = dest.strip_prefix(work_dir).unwrap_or(&dest).display();
                            let tag = if multi {
                                format!("[{}] ", gv.package_version)
                            } else {
                                String::new()
                            };
                            ui::detail(format!("     {tag}{src_name} {} {shown}", ui::arrow()));
                        }
                        destinations.push(dest);
                        written += 1;
                    }
                }
                // A crate at `<out>/<language>/` by default, or straight into
                // `out` with `flatten = true`; the generator owns the layout
                // inside it either way — `layout` itself never applies here.
                Mode::Lib => {
                    let root = if t.flatten {
                        t.out.clone()
                    } else {
                        t.out.join(&t.language)
                    };
                    for f in &files {
                        let dest = root.join(f.path_buf());
                        if !dry_run {
                            write_file(&dest, &f.contents)?;
                        }
                        if list {
                            let shown = dest.strip_prefix(work_dir).unwrap_or(&dest).display();
                            ui::detail(format!("     {} {} {shown}", f.path, ui::arrow()));
                        }
                        destinations.push(dest);
                        written += 1;
                    }
                }
            }

            if !list {
                ui::tree(work_dir, &destinations);
            }
        }
    }

    ui::success(if dry_run {
        format!("Would write {written} file(s)")
    } else {
        format!("Generated {written} file(s)")
    });
    Ok(())
}

/// Which generator a target runs.
enum Source<'e> {
    Builtin(Resolved),
    External(&'e External),
}

/// External generators by path and timeout, each opened and described once
/// per run.
#[derive(Default)]
struct Externals {
    described: Vec<((PathBuf, Duration), (External, Description))>,
}

impl Externals {
    fn get(&mut self, path: &Path, timeout: Duration) -> Result<(&External, &Description)> {
        let key = (path.to_owned(), timeout);
        let at = match self.described.iter().position(|(k, _)| *k == key) {
            Some(at) => at,
            None => {
                let generator = External::open(path, timeout)?;
                let description = generator.describe()?;
                self.described.push((key, (generator, description)));
                self.described.len() - 1
            }
        };
        let (generator, description) = &self.described[at].1;
        Ok((generator, description))
    }
}

fn write_file(dest: &Path, contents: &[u8]) -> Result<()> {
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
    live_config: &[ConfigUnit],
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
                wire_format: frozen::wire_format(live_config).to_string(),
            }])
        }
        VersionSpec::All => {
            // Every release: what consumers see. The builds between them
            // are named one by one when wanted.
            let chain = history::load(work_dir)?;
            let store = ObjectStore::new(work_dir);
            let releases: Vec<_> = chain.iter().filter(|e| e.commit.is_release()).collect();
            if releases.is_empty() {
                return Err(miette!(
                    "`package_versions = \"all\"` generates every release, and there's none yet; \
                     run `comline release`, or name the builds to generate"
                ));
            }
            releases
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
    let config = history::load_config(store, commit)?;
    Ok(GenVersion {
        package_version: version.to_owned(),
        schemas,
        wire_format: frozen::wire_format(&config).to_string(),
    })
}

/// `async = true` changes only protocols on a connection; with none, the
/// output is the same as without it, which looks like the setting did nothing.
/// Every protocol is on one by default, so that's a package whose protocols
/// are all lockstep: on the `datagram` framing, or on JSON-RPC with none of
/// them client-provided.
fn warn_async_unused(language: &str, request: &GenerateRequest) {
    if !request.async_api {
        return;
    }
    let uses = features::used(&request.schemas, &features::Options::of(request));
    if uses.iter().any(|u| u.feature == features::Feature::Async) {
        return;
    }
    ui::warn(format!(
        "`{language}`: `async = true`, but no protocol runs on a connection, so the output \
         is the same as without it - every protocol here is lockstep: on the `datagram` \
         framing (the default, `datagram@2`, is a connection), or on JSON-RPC with none of \
         the package's JSON-RPC protocols client-provided"
    ));
}

/// The package's `serve` / `connect` need its protocols on one connection;
/// when they disagree on the framing or handshake, the generators leave them
/// out, which would otherwise look like they forgot.
fn warn_no_package_connection(language: &str, request: &GenerateRequest) {
    if let Err(reason) = connections::package_connection(&request.schemas) {
        ui::warn(format!(
            "`{language}`: no package `serve` / `connect` - {reason}; each protocol still has \
             its own"
        ));
    }
}

/// A warning per feature `request` uses that this generator ignores - it
/// generates, but without the feature (a TypeScript target with
/// `@validators` gets the types, not the checks). An unsupported feature
/// needs nothing here: the generator refuses it, naming every use.
fn warn_ignored(language: &str, request: &GenerateRequest, resolved: &Resolved) {
    let uses = features::used(&request.schemas, &features::Options::of(request));
    let mut ignored: BTreeMap<features::Feature, Vec<&features::FeatureUse>> = BTreeMap::new();
    for u in &uses {
        if resolved.support(u.feature).state == features::State::Ignored {
            ignored.entry(u.feature).or_default().push(u);
        }
    }
    for (feature, uses) in ignored {
        let s = resolved.support(feature);
        let mut message = format!("`{language}` ignores {}", feature.title());
        if let Some(note) = s.note {
            message.push_str(&format!(": {note}"));
        }
        if let Some(tracking) = s.tracking {
            message.push_str(&format!(" ({tracking})"));
        }
        let first = uses[0];
        let place = match first.namespace.is_empty() {
            true => first.owner.clone(),
            false => format!("schema '{}', {}", first.namespace, first.owner),
        };
        message.push_str(&format!(" - used by {place}"));
        if uses.len() > 1 {
            message.push_str(&format!(" and {} more", uses.len() - 1));
        }
        ui::warn(message);
    }
}
