//! What a package's declared targets promise, and the checks that hold it
//! to them.
//!
//! `config.idp` declares each language a package generates for, with the
//! oldest toolchain its generated code must build on: `rust#1.85.0`. That
//! declaration is a promise consumers rely on, so `comline build` proves
//! every one before a version is frozen ([`check_declared`]), and
//! `comline generate` holds each target to the same rules ([`resolve`]):
//!
//! - A generator in this binary handles the version: not below its floor,
//!   and dotted numbers.
//! - It's no lower than any dependency's declaration for the language. A
//!   dependency's generated code needs at least its own toolchain.
//! - Every feature the schemas use is supported at that version.
//! - Past what the generator has verified, it warns under
//!   [`UNVERIFIED_VERSION`], which a package's settings can raise to an
//!   error (`diagnostics."generation::unverified-version".level`).
//!
//! See the *Language target versions* proposal in the docs.

use comline_codegen::features::{self, Options};
use comline_codegen::versions::LangVersion;
use comline_codegen::{Registry, Resolved, Unresolved};
use comline_core::diagnostics::{Diagnostic, DiagnosticCode};
use comline_core::package::config::ir::context::ProjectContext;
use comline_core::package::config::ir::frozen::{self, FrozenUnit as ConfigUnit};
use comline_core::schema::ir::compiler::settings::severity::resolve_diagnostic_overrides;
use comline_core::schema::ir::frozen::unit::FrozenUnit;
use comline_core::settings::effective::package_settings;
use comline_core::settings::SettingsSpans;

use crate::gen_config::DeclaredLang;

/// A target declared or raised past the newest version its generator has
/// verified. A warning: the wire isn't affected, only whoever compiles the
/// output, so there's nothing to accept.
pub const UNVERIFIED_VERSION: DiagnosticCode =
    DiagnosticCode::warning("generation::unverified-version");

/// The languages a congregation's config declares: `rust#1.85.0` ->
/// (`rust`, `1.85.0`).
pub fn declared(config: &[ConfigUnit]) -> Vec<DeclaredLang> {
    config
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
        .collect()
}

/// The highest version any dependency, direct or not, declares for
/// `language`: (the dependency's name, its declaration as written, parsed).
/// An unparsable one is that dependency's own build's business.
fn dependency_minimum(
    context: &ProjectContext,
    language: &str,
) -> Option<(String, String, LangVersion)> {
    context
        .dependency_configs
        .iter()
        .flat_map(|(dependency, config)| {
            declared(config)
                .into_iter()
                .filter(|d| d.language.eq_ignore_ascii_case(language))
                .filter_map(move |d| {
                    let version = LangVersion::parse(&d.lang_version)?;
                    Some((dependency.clone(), d.lang_version, version))
                })
        })
        .max_by(|a, b| a.2.cmp(&b.2))
}

/// `language#version`'s generator, held to the package's dependencies: an
/// error below the generator's floor, for a version that isn't one, for a
/// language this binary can't generate, or below a dependency's declaration.
pub fn resolve(
    registry: &Registry,
    context: &ProjectContext,
    language: &str,
    version: &str,
) -> Result<Resolved, TargetError> {
    let resolved = registry
        .resolve(language, version)
        .map_err(TargetError::Unresolved)?;
    if let Some((dependency, written, minimum)) = dependency_minimum(context, language) {
        if resolved.version < minimum {
            return Err(TargetError::BelowDependency(format!(
                "`{language}#{version}` is older than dependency `{dependency}` allows: it declares \
                 `{language}#{written}`, and its generated code needs that toolchain - declare \
                 `{language}#{written}` or later"
            )));
        }
    }
    Ok(resolved)
}

/// Why a target can't be generated.
pub enum TargetError {
    Unresolved(Unresolved),
    BelowDependency(String),
}

impl std::fmt::Display for TargetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TargetError::Unresolved(unresolved) => unresolved.fmt(f),
            TargetError::BelowDependency(message) => f.write_str(message),
        }
    }
}

/// What `schemas` use that `resolved` doesn't support at its version, as the
/// generator's own refusal would put it. `None` when there's nothing.
pub fn unsupported(
    language: &str,
    resolved: &Resolved,
    schemas: &[(String, Vec<FrozenUnit>)],
    options: &Options,
) -> Option<String> {
    let support = resolved.generator.support;
    let uses = features::used(schemas, options);
    let unsupported = features::unsupported_uses(&uses, support, &resolved.version);
    (!unsupported.is_empty())
        .then(|| features::refusal(language, &unsupported, support, &resolved.version))
}

/// The [`UNVERIFIED_VERSION`] warning for `resolved`, when it's past what its
/// generator has verified, with the package's diagnostics settings applied:
/// `Ok(Some(_))` to print, `Ok(None)` when verified or silenced, `Err(_)`
/// when the settings make it an error.
pub fn unverified(
    context: &ProjectContext,
    language: &str,
    resolved: &Resolved,
) -> Result<Option<String>, String> {
    if resolved.is_verified() {
        return Ok(None);
    }
    let versions = resolved.generator.versions;
    let message = format!(
        "`{language}#{}` is past what the {language} generator has verified (up to {}): it generates \
         as for {}, and the output may not build on that toolchain ({})",
        resolved.version,
        versions.verified_up_to,
        resolved.boundary(),
        UNVERIFIED_VERSION.id
    );
    let diagnostic = Diagnostic::new(UNVERIFIED_VERSION, message.clone());
    let (warnings, fatal) = resolve_diagnostic_overrides(
        &[],
        vec![diagnostic],
        &package_settings(context),
        &SettingsSpans::default(),
    );
    if let Some(error) = fatal.first() {
        return Err(format!(
            "{} - `diagnostics.\"{}\".level` makes it an error",
            error.message, UNVERIFIED_VERSION.id
        ));
    }
    Ok(warnings.first().map(|_| message))
}

/// `comline build`'s check: every language `context` declares, proven at its
/// declared version. Returns the warnings to print; every problem is
/// gathered into one error, so a package with several sees them all at once.
pub fn check_declared(context: &ProjectContext, registry: &Registry) -> eyre::Result<Vec<String>> {
    let config = context.config_frozen.as_deref().unwrap_or_default();
    let schemas: Vec<(String, Vec<FrozenUnit>)> = context
        .schema_contexts
        .iter()
        .filter_map(|sc| {
            let sc = sc.borrow();
            let units = sc.frozen_schema.borrow().clone()?;
            Some((sc.namespace.join("/"), units))
        })
        .collect();

    let mut problems = Vec::new();
    let mut warnings = Vec::new();
    for d in declared(config) {
        let resolved = match resolve(registry, context, &d.language, &d.lang_version) {
            Ok(resolved) => resolved,
            Err(TargetError::Unresolved(Unresolved::UnknownLanguage { language })) => {
                problems.push(format!(
                    "`{language}#{}` can't be checked: this build of comline has no {language} generator \
                     - build with one that does, or remove the declaration",
                    d.lang_version
                ));
                continue;
            }
            Err(e) => {
                problems.push(e.to_string());
                continue;
            }
        };
        // Options a consumer picks in `comline.toml` aren't the package's
        // promise, so only what the schemas and config.idp use is checked.
        let options = Options {
            wire_format: Some(frozen::wire_format(config).to_string()),
            ..Options::default()
        };
        if let Some(refusal) = unsupported(&d.language, &resolved, &schemas, &options) {
            problems.push(format!("`{}#{}`: {refusal}", d.language, d.lang_version));
        }
        match unverified(context, &d.language, &resolved) {
            Ok(Some(warning)) => warnings.push(warning),
            Ok(None) => {}
            Err(error) => problems.push(error),
        }
    }

    // JSON-RPC protocols on one connection whose bare method names a side
    // couldn't tell apart: no generator can emit them.
    for clash in comline_codegen::connections::bare_name_clashes(&schemas) {
        problems.push(format!("every target: {clash}"));
    }

    if problems.is_empty() {
        Ok(warnings)
    } else {
        Err(eyre::eyre!(
            "the package's declared targets don't hold (config.idp, `code_generation.languages`):\n\n{}",
            problems.join("\n\n")
        ))
    }
}
