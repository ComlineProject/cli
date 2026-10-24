//! `comline.toml` — the consumer-owned code-generation config.
//!
//! `config.idp` (the congregation) declares *what* a package can be generated as
//! (`code_generation.languages`); it is frozen into the package and must not
//! carry output paths. `comline.toml` is the other half: it belongs to whoever
//! *consumes* the package and says where generated code lands, in what layout,
//! and in what form. It is plain committed source — never frozen, never in CAS.
//!
//! Field precedence, lowest to highest: built-in defaults → `[generate]` →
//! `[[generate.target]]` → `COMLINE_GENERATE_*` env → CLI flags.

use std::path::{Path, PathBuf};

use comline_codegen::validation::ValidateOnDecode;
use comline_codegen::versions::LangVersion;
use comline_core::schema::idl::annotations;
use comline_core::utils::templating::recurse_render;
use comline_core::utils::text::closest;
use miette::{miette, IntoDiagnostic, Result, WrapErr};
use serde::{Deserialize, Serialize};

/// File looked for in the project directory.
pub const FILE_NAME: &str = "comline.toml";

/// Default output root, relative to `comline.toml`.
pub const DEFAULT_OUT: &str = "generated";
/// Default on-disk layout under a target's root.
pub const DEFAULT_LAYOUT: &str = "{{language}}/{{namespace}}.{{ext}}";
/// Default emit form. `"code"` and `"lib"` are implemented; `"dylib"` is not.
pub const DEFAULT_MODE: &str = "code";

/// Whether `std` is referenced as its own package rather than inlined, when
/// nothing says either way.
///
/// It follows the mode, because the two modes differ in whether there is
/// anywhere to put the dependency. `lib` writes the package manifest, so the
/// generator adds `comline_std` to it and the crate links one shared std.
/// `code` emits loose source files with no manifest to add anything to - all
/// a generator can do there is leave a comment naming the dependency line to
/// write by hand, so inlining stays the honest default.
fn default_external_std(mode: &str) -> bool {
    mode == "lib"
}

/// Which package versions to generate bindings for.
///
/// TOML: `package_versions = "latest"` (default) | `"all"` | `"0.3.0"` |
/// `["0.3.0", "0.4.0"]`. `latest` is the working tree; anything else reads
/// committed versions from the CAS chain: `all` every release, a list any
/// version named, a build (`0.3.0-dev.2`) included.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum VersionSpec {
    /// The working tree (no CAS read).
    #[default]
    Latest,
    /// Every committed version in the chain.
    All,
    /// Specific committed versions (each a version string or commit hash).
    List(Vec<String>),
}

impl<'de> Deserialize<'de> for VersionSpec {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            One(String),
            Many(Vec<String>),
        }
        Ok(match Raw::deserialize(d)? {
            Raw::One(s) if s.eq_ignore_ascii_case("latest") => VersionSpec::Latest,
            Raw::One(s) if s.eq_ignore_ascii_case("all") => VersionSpec::All,
            Raw::One(s) => VersionSpec::List(vec![s]),
            Raw::Many(v) => VersionSpec::List(v),
        })
    }
}

/// Parsed `comline.toml`. An absent file parses as [`Self::default`].
#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComlineToml {
    #[serde(default)]
    pub generate: Generate,
}

/// The `[generate]` table.
#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Generate {
    /// Narrow generation to these languages, out of the ones `config.idp`
    /// declares. Empty (the default) means every declared language.
    ///
    /// The consumer's half of the split: `config.idp` says which languages
    /// the *package* is for, and this says which of them *this checkout*
    /// wants built. A name the congregation doesn't declare is an error
    /// rather than a silent no-op, so a typo here can't quietly stop
    /// generating something.
    pub only: Vec<String>,
    /// Output root shared by every target unless overridden.
    pub out: Option<String>,
    /// Layout template shared by every target unless overridden.
    pub layout: Option<String>,
    /// Emit form shared by every target unless overridden.
    pub mode: Option<String>,
    /// Version selection shared by every target unless overridden.
    pub package_versions: Option<VersionSpec>,
    /// Fallback wire framing for protocols that do not pick one with `@framing`,
    /// shared by every target unless overridden. One of `@framing`'s accepted
    /// spellings (`"jsonrpc"`, `"datagram"`, …), folded onto its canonical
    /// form; an unknown one is an error.
    pub default_framing: Option<String>,
    /// `true`: reference `comline_std` as its own crate instead of inlining
    /// it, shared by every target unless overridden. Only a generator that
    /// knows how (today, Rust's) acts on it. Default `false`.
    pub external_std: Option<bool>,
    /// Whether decoding a value runs its `@validators` checks: `off` (the
    /// default), `collect` (fail with every failure) or `first` (fail with
    /// the first). Shared by every target unless overridden.
    pub validate_on_decode: Option<String>,
    /// `[[generate.target]]` blocks.
    #[serde(rename = "target")]
    pub targets: Vec<Target>,
}

/// One `[[generate.target]]` block.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    /// Must match a language the congregation declared under
    /// `code_generation.languages`.
    pub language: String,
    /// Language/toolchain version selecting the generator (e.g. `1.88.0`).
    /// Falls back to the version the congregation declared for this language,
    /// and may raise it but never lower it.
    pub lang_version: Option<String>,
    pub out: Option<String>,
    pub layout: Option<String>,
    pub mode: Option<String>,
    pub package_versions: Option<VersionSpec>,
    pub default_framing: Option<String>,
    pub external_std: Option<bool>,
    pub validate_on_decode: Option<String>,
    /// `lib` mode only: write the crate straight into `out` instead of
    /// `<out>/<language>/`. Target-only (no `[generate]` default, no flag/env)
    /// — a shared `out` across several `lib` targets needs the per-language
    /// split to avoid two crates colliding in one directory, so flattening
    /// only makes sense once a target already has `out` to itself.
    #[serde(default)]
    pub flatten: bool,
}

/// A non-empty `COMLINE_GENERATE_*` env var, if set.
fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
}

/// Same as [`env_var`], parsed as a bool (`"true"`/`"1"` ⇒ on,
/// `"false"`/`"0"` ⇒ off; anything else unset, same as absent).
fn env_var_bool(name: &str) -> Option<bool> {
    match env_var(name)?.as_str() {
        "true" | "1" => Some(true),
        "false" | "0" => Some(false),
        _ => None,
    }
}

impl ComlineToml {
    /// Load `<work_dir>/comline.toml`; a missing file yields defaults.
    pub fn load(work_dir: &Path) -> Result<Self> {
        let path = work_dir.join(FILE_NAME);
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text)
                .into_diagnostic()
                .wrap_err_with(|| format!("failed to parse `{}`", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e)
                .into_diagnostic()
                .wrap_err_with(|| format!("failed to read `{}`", path.display())),
        }
    }
}

/// A language the congregation declared, `name#version` already split.
#[derive(Debug, Clone)]
pub struct DeclaredLang {
    pub language: String,
    pub lang_version: String,
}

/// CLI flag overrides for `comline generate`.
#[derive(Debug, Default)]
pub struct Overrides<'a> {
    pub target: Option<&'a str>,
    pub out: Option<&'a str>,
    pub layout: Option<&'a str>,
    pub mode: Option<&'a str>,
    /// `Some(true)` from `--external-std`, `Some(false)` from
    /// `--no-external-std`, `None` when neither was passed. Both spellings
    /// exist because the default is no longer a fixed `false` - it follows
    /// the mode (see [`default_external_std`]), so either direction can be
    /// the one a given run needs to override.
    pub external_std: Option<bool>,
    /// `--validate-on-decode`.
    pub validate_on_decode: Option<&'a str>,
}

/// A fully-resolved target: every field decided, `out` made absolute.
#[derive(Debug)]
pub struct ResolvedTarget {
    pub language: String,
    pub lang_version: String,
    pub out: PathBuf,
    pub layout: String,
    pub mode: String,
    pub versions: VersionSpec,
    /// Fallback framing handed to the generator; `None` ⇒ its built-in default.
    pub default_framing: Option<String>,
    /// `lib` mode only — see [`Target::flatten`].
    pub flatten: bool,
    /// See [`Generate::external_std`].
    pub external_std: bool,
    /// See [`Generate::validate_on_decode`].
    pub validate_on_decode: ValidateOnDecode,
}

/// Variables a `layout` template can reference.
#[derive(Serialize)]
struct LayoutVars<'a> {
    language: &'a str,
    namespace: &'a str,
    ext: &'a str,
    lang_version: &'a str,
    spec_version: &'a str,
    package_version: &'a str,
}

impl ResolvedTarget {
    /// Render `layout` for one schema namespace into an absolute path under
    /// `self.out`. `ext` is the generator's file extension (e.g. `rs`);
    /// `package_version` is the version being generated (empty for an unbuilt
    /// working tree).
    pub fn dest_for(
        &self,
        namespace: &str,
        ext: &str,
        spec_version: &str,
        package_version: &str,
    ) -> Result<PathBuf> {
        let vars = LayoutVars {
            language: &self.language,
            namespace,
            ext,
            lang_version: &self.lang_version,
            spec_version,
            package_version,
        };
        let rel = recurse_render(&self.layout, &vars)
            .map_err(|e| miette!("bad `layout` template `{}`: {e}", self.layout))?;
        Ok(self.out.join(rel))
    }
}

/// Merge `comline.toml`, the congregation's declared languages and CLI flags
/// into the concrete set of targets `generate` should emit.
///
/// **The declared languages are the set.** `config.idp`'s
/// `code_generation.languages` says which languages this package is generated
/// for - that is the package's own contract, committed alongside it - and a
/// `[[generate.target]]` block says where and how one of them lands. A block
/// used to *replace* the list outright, so adding one to redirect `rust`'s
/// output silently stopped generating every other declared language; the
/// docs described the block as settings all along ("one block per target you
/// want to pin or override"), and this is the code catching up.
///
/// A language with no block takes the `[generate]` defaults. Several blocks
/// for one language give several targets - two `rust` blocks writing to
/// different `out` is a real thing to want. A block naming a language the
/// congregation doesn't declare is an error, not a silent no-op: that would
/// be the same footgun inverted.
///
/// With no declared languages at all, the blocks are the set - a
/// `comline.toml`-only project with no `code_generation` in its congregation
/// still works.
///
/// `[generate] only` narrows the declared set persistently; `--target`
/// narrows it for one run (case-insensitive). The `--out` / `--layout` /
/// `--mode` flags apply to the single remaining target, or to the
/// `--target`-named one; using them with several targets and no `--target` is an
/// error. `COMLINE_GENERATE_{OUT,LAYOUT,MODE}` sit just below the flags but apply
/// to every target regardless (a deliberate global for CI).
pub fn resolve(
    cfg: &ComlineToml,
    declared: &[DeclaredLang],
    work_dir: &Path,
    ov: &Overrides,
) -> Result<Vec<ResolvedTarget>> {
    let declares = |language: &str| {
        declared
            .iter()
            .any(|d| d.language.eq_ignore_ascii_case(language))
    };

    // A block for a language nobody declared is a mistake worth naming -
    // it would otherwise configure a target that is never built.
    if !declared.is_empty() {
        if let Some(stray) = cfg.generate.targets.iter().find(|t| !declares(&t.language)) {
            return Err(miette!(
                "`[[generate.target]]` names `{}`, which config.idp does not declare \
                 - add `{}#<version> = {{}}` to `code_generation.languages`, or drop \
                 the block",
                stray.language,
                stray.language,
            ));
        }
    }
    if let Some(stray) = cfg
        .generate
        .only
        .iter()
        .find(|l| !declared.is_empty() && !declares(l))
    {
        return Err(miette!(
            "`[generate] only` names `{stray}`, which config.idp does not declare",
        ));
    }

    let base: Vec<Target> = if declared.is_empty() {
        // No congregation-declared languages: the blocks are all there is.
        cfg.generate.targets.clone()
    } else {
        declared
            .iter()
            .filter(|d| {
                cfg.generate.only.is_empty()
                    || cfg
                        .generate
                        .only
                        .iter()
                        .any(|l| l.eq_ignore_ascii_case(&d.language))
            })
            .flat_map(|d| {
                let blocks: Vec<Target> = cfg
                    .generate
                    .targets
                    .iter()
                    .filter(|t| t.language.eq_ignore_ascii_case(&d.language))
                    .cloned()
                    .collect();
                if blocks.is_empty() {
                    // Declared but unconfigured: the `[generate]` defaults.
                    vec![Target {
                        language: d.language.clone(),
                        lang_version: Some(d.lang_version.clone()),
                        out: None,
                        layout: None,
                        mode: None,
                        package_versions: None,
                        default_framing: None,
                        external_std: None,
                        validate_on_decode: None,
                        flatten: false,
                    }]
                } else {
                    blocks
                }
            })
            .collect()
    };

    let selected: Vec<&Target> = base
        .iter()
        .filter(|t| match ov.target {
            Some(want) => want.eq_ignore_ascii_case(&t.language),
            None => true,
        })
        .collect();

    if selected.is_empty() {
        return match ov.target {
            Some(want) => Err(miette!("no code-generation target matches `{want}`")),
            None if !cfg.generate.only.is_empty() => {
                Err(miette!("`[generate] only` leaves nothing to generate",))
            }
            None => Err(miette!(
                "nothing to generate: add `[[generate.target]]` to {} \
                 or `code_generation.languages` to config.idp",
                FILE_NAME
            )),
        };
    }

    let field_override = ov.out.is_some()
        || ov.layout.is_some()
        || ov.mode.is_some()
        || ov.external_std.is_some()
        || ov.validate_on_decode.is_some();
    if field_override && selected.len() > 1 && ov.target.is_none() {
        return Err(miette!(
            "`--out` / `--layout` / `--mode` / `--external-std` / `--validate-on-decode` \
             need `--target <lang>` when more than one target is configured ({} here)",
            selected.len()
        ));
    }
    // A flag binds to a target when it was explicitly named, or when it is the
    // only one left. `COMLINE_*` env vars are a deliberate global — they apply to
    // every target regardless.
    let apply_flag = ov.target.is_some() || selected.len() == 1;
    let env_out = env_var("COMLINE_GENERATE_OUT");
    let env_layout = env_var("COMLINE_GENERATE_LAYOUT");
    let env_mode = env_var("COMLINE_GENERATE_MODE");
    let env_external_std = env_var_bool("COMLINE_GENERATE_EXTERNAL_STD");
    let env_validate_on_decode = env_var("COMLINE_GENERATE_VALIDATE_ON_DECODE");

    let mut resolved = Vec::with_capacity(selected.len());
    for t in selected {
        // Precedence, highest first: flag → env → [[target]] → [generate] → default.
        let pick = |flag: Option<&str>,
                    env: &Option<String>,
                    target: &Option<String>,
                    section: &Option<String>,
                    default: &str| {
            flag.filter(|_| apply_flag)
                .map(str::to_owned)
                .or_else(|| env.clone())
                .or_else(|| target.clone())
                .or_else(|| section.clone())
                .unwrap_or_else(|| default.to_owned())
        };

        // Same precedence, for a bool field.
        let pick_bool = |flag: Option<bool>,
                         env: Option<bool>,
                         target: Option<bool>,
                         section: Option<bool>,
                         default: bool| {
            flag.filter(|_| apply_flag)
                .or(env)
                .or(target)
                .or(section)
                .unwrap_or(default)
        };

        let out = pick(ov.out, &env_out, &t.out, &cfg.generate.out, DEFAULT_OUT);
        let layout = pick(
            ov.layout,
            &env_layout,
            &t.layout,
            &cfg.generate.layout,
            DEFAULT_LAYOUT,
        );
        let mode = pick(
            ov.mode,
            &env_mode,
            &t.mode,
            &cfg.generate.mode,
            DEFAULT_MODE,
        );
        let external_std = pick_bool(
            ov.external_std,
            env_external_std,
            t.external_std,
            cfg.generate.external_std,
            default_external_std(&mode),
        );
        let validate_on_decode = pick(
            ov.validate_on_decode,
            &env_validate_on_decode,
            &t.validate_on_decode,
            &cfg.generate.validate_on_decode,
            ValidateOnDecode::Off.name(),
        );
        let validate_on_decode = ValidateOnDecode::parse(&validate_on_decode).ok_or_else(|| {
            miette!(
                "`validate_on_decode = \"{validate_on_decode}\"` for target `{}` - expected one of \
                 {}",
                t.language,
                ValidateOnDecode::NAMES.map(|n| format!("`{n}`")).join(", ")
            )
        })?;

        let declared_version = declared
            .iter()
            .find(|d| d.language.eq_ignore_ascii_case(&t.language))
            .map(|d| d.lang_version.clone());
        // `comline.toml` may raise the version, never lower it: config.idp's
        // is the oldest toolchain the package was proven against. A version
        // that doesn't parse is left for the registry to name.
        if let (Some(raised), Some(declared_version)) = (&t.lang_version, &declared_version) {
            if let (Some(r), Some(d)) = (
                LangVersion::parse(raised),
                LangVersion::parse(declared_version),
            ) {
                if r < d {
                    return Err(miette!(
                        "target `{}` sets `lang_version = \"{raised}\"`, below config.idp's `{}#{declared_version}` \
                         - {} can raise a target's version, never lower it: the package is only proven \
                         from `{declared_version}` on",
                        t.language,
                        t.language,
                        FILE_NAME
                    ));
                }
            }
        }
        let lang_version = t.lang_version.clone().or(declared_version).ok_or_else(|| {
            miette!(
                "target `{}` sets no `lang_version` and config.idp does not \
                     declare that language under `code_generation.languages`",
                t.language
            )
        })?;

        let versions = t
            .package_versions
            .clone()
            .or_else(|| cfg.generate.package_versions.clone())
            .unwrap_or_default();

        // `[[target]]` wins over `[generate]`; `None` at both ⇒ the generator's
        // built-in default. No flag / env override — schema-shaping config, not
        // a per-run knob.
        let default_framing = t
            .default_framing
            .as_deref()
            .or(cfg.generate.default_framing.as_deref())
            .map(|written| canonical_framing(written, &t.language))
            .transpose()?;

        resolved.push(ResolvedTarget {
            language: t.language.clone(),
            lang_version,
            out: work_dir.join(out),
            layout,
            mode,
            versions,
            default_framing,
            flatten: t.flatten,
            external_std,
            validate_on_decode,
        });
    }

    Ok(resolved)
}

/// Fold a `default_framing` onto the canonical spelling `@framing` uses, the
/// same table core checks `@framing` against. An unknown name is an error:
/// each generator falls back to datagram for a name it doesn't recognise, so
/// a typo would otherwise generate the wrong framing and only surface as a
/// refused handshake.
fn canonical_framing(written: &str, language: &str) -> Result<String> {
    if let Some(canonical) = annotations::canonical_value("framing", written) {
        return Ok(canonical.to_string());
    }
    let spellings = annotations::value_spellings("framing");
    let hint = closest(written, spellings.iter().copied())
        .map(|s| format!(" - did you mean `{s}`?"))
        .unwrap_or_default();
    Err(miette!(
        "`default_framing = \"{written}\"` for target `{language}` - expected one of {}{hint}",
        spellings
            .iter()
            .map(|n| format!("`{n}`"))
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(toml_src: &str) -> ComlineToml {
        toml::from_str(toml_src).unwrap()
    }

    #[test]
    fn version_spec_forms() {
        assert_eq!(
            parse("[generate]\npackage_versions = \"latest\"")
                .generate
                .package_versions,
            Some(VersionSpec::Latest)
        );
        assert_eq!(
            parse("[generate]\npackage_versions = \"ALL\"")
                .generate
                .package_versions,
            Some(VersionSpec::All)
        );
        assert_eq!(
            parse("[generate]\npackage_versions = \"0.3.0\"")
                .generate
                .package_versions,
            Some(VersionSpec::List(vec!["0.3.0".into()]))
        );
        assert_eq!(
            parse("[generate]\npackage_versions = [\"0.3.0\", \"0.4.0\"]")
                .generate
                .package_versions,
            Some(VersionSpec::List(vec!["0.3.0".into(), "0.4.0".into()]))
        );
        assert_eq!(parse("[generate]").generate.package_versions, None);
    }

    #[test]
    fn target_package_versions_wins_over_section() {
        let cfg = parse(
            "[generate]\npackage_versions = \"all\"\n\n\
             [[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\n\
             package_versions = \"latest\"\n",
        );
        let declared = [];
        let resolved = resolve(&cfg, &declared, Path::new("/x"), &Overrides::default()).unwrap();
        assert_eq!(resolved[0].versions, VersionSpec::Latest);
    }

    fn two_langs() -> [DeclaredLang; 2] {
        [
            DeclaredLang {
                language: "rust".into(),
                lang_version: "1.85.0".into(),
            },
            DeclaredLang {
                language: "typescript".into(),
                lang_version: "5.0".into(),
            },
        ]
    }

    /// The bug this rule replaced: one block existed only to redirect
    /// `rust`'s output, and every other declared language silently stopped
    /// being generated.
    #[test]
    fn a_block_configures_its_language_without_dropping_the_others() {
        let cfg = parse("[[generate.target]]\nlanguage = \"rust\"\nout = \"../test-cl-rs/\"\n");
        let resolved = resolve(&cfg, &two_langs(), Path::new("/x"), &Overrides::default()).unwrap();

        let langs: Vec<&str> = resolved.iter().map(|t| t.language.as_str()).collect();
        assert_eq!(langs, vec!["rust", "typescript"]);
        assert_eq!(
            resolved[0].out,
            Path::new("/x/../test-cl-rs/"),
            "the block's out"
        );
        assert_eq!(
            resolved[1].out,
            Path::new("/x/generated"),
            "the [generate] default"
        );
        // A language with no block still gets the version its congregation declared.
        assert_eq!(resolved[1].lang_version, "5.0");
    }

    #[test]
    fn a_block_may_be_matched_case_insensitively() {
        let cfg = parse("[[generate.target]]\nlanguage = \"Rust\"\nout = \"elsewhere\"\n");
        let resolved = resolve(&cfg, &two_langs(), Path::new("/x"), &Overrides::default()).unwrap();
        assert_eq!(resolved[0].out, Path::new("/x/elsewhere"));
    }

    /// Two outputs for one language is a real thing to want, and survives
    /// the declared set becoming the base.
    #[test]
    fn several_blocks_for_one_language_give_several_targets() {
        let cfg = parse(
            "[[generate.target]]\nlanguage = \"rust\"\nout = \"a\"\n\n\
             [[generate.target]]\nlanguage = \"rust\"\nout = \"b\"\n",
        );
        let declared = [DeclaredLang {
            language: "rust".into(),
            lang_version: "1.85.0".into(),
        }];
        let resolved = resolve(&cfg, &declared, Path::new("/x"), &Overrides::default()).unwrap();
        assert_eq!(resolved.len(), 2);
        assert_eq!(resolved[0].out, Path::new("/x/a"));
        assert_eq!(resolved[1].out, Path::new("/x/b"));
    }

    /// Silently ignoring a block someone wrote would be the same footgun
    /// inverted, so it names the fix instead.
    #[test]
    fn a_block_for_an_undeclared_language_is_an_error() {
        let cfg = parse("[[generate.target]]\nlanguage = \"python\"\nlang_version = \"3.11\"\n");
        let declared = [DeclaredLang {
            language: "rust".into(),
            lang_version: "1.85.0".into(),
        }];
        let err = resolve(&cfg, &declared, Path::new("/x"), &Overrides::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("config.idp does not declare"), "{err}");
        assert!(err.contains("code_generation.languages"), "{err}");
    }

    /// A congregation with no `code_generation` at all: the blocks are all
    /// there is, as they always were.
    #[test]
    fn blocks_stand_alone_when_nothing_is_declared() {
        let cfg = parse("[[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\n");
        let resolved = resolve(&cfg, &[], Path::new("/x"), &Overrides::default()).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].language, "rust");
    }

    #[test]
    fn only_narrows_the_declared_set() {
        let cfg = parse("[generate]\nonly = [\"rust\"]\n");
        let resolved = resolve(&cfg, &two_langs(), Path::new("/x"), &Overrides::default()).unwrap();
        let langs: Vec<&str> = resolved.iter().map(|t| t.language.as_str()).collect();
        assert_eq!(langs, vec!["rust"]);
    }

    /// A typo in `only` would otherwise quietly stop generating something.
    #[test]
    fn only_naming_an_undeclared_language_is_an_error() {
        let cfg = parse("[generate]\nonly = [\"rustt\"]\n");
        let err = resolve(&cfg, &two_langs(), Path::new("/x"), &Overrides::default())
            .unwrap_err()
            .to_string();
        assert!(err.contains("does not declare"), "{err}");
    }

    #[test]
    fn only_still_lets_a_block_configure_what_survives() {
        let cfg = parse(
            "[generate]\nonly = [\"rust\"]\n\n\
             [[generate.target]]\nlanguage = \"rust\"\nout = \"here\"\n\n\
             [[generate.target]]\nlanguage = \"typescript\"\nout = \"there\"\n",
        );
        let resolved = resolve(&cfg, &two_langs(), Path::new("/x"), &Overrides::default()).unwrap();
        assert_eq!(resolved.len(), 1);
        assert_eq!(resolved[0].out, Path::new("/x/here"));
    }

    #[test]
    fn default_framing_section_reaches_a_declared_target() {
        let cfg = parse("[generate]\ndefault_framing = \"jsonrpc\"\n");
        let declared = [DeclaredLang {
            language: "rust".into(),
            lang_version: "1.80".into(),
        }];
        let resolved = resolve(&cfg, &declared, Path::new("/x"), &Overrides::default()).unwrap();
        assert_eq!(resolved[0].default_framing.as_deref(), Some("jsonrpc"));
    }

    #[test]
    fn target_default_framing_wins_over_section() {
        let cfg = parse(
            "[generate]\ndefault_framing = \"jsonrpc\"\n\n\
             [[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\n\
             default_framing = \"datagram\"\n",
        );
        let declared = [];
        let resolved = resolve(&cfg, &declared, Path::new("/x"), &Overrides::default()).unwrap();
        assert_eq!(resolved[0].default_framing.as_deref(), Some("datagram"));
    }

    /// `lib` writes the package manifest, so the generator can add
    /// `comline_std` to it; `code` emits loose files with nowhere to put a
    /// dependency, so inlining stays the honest default there.
    #[test]
    fn external_std_follows_the_mode() {
        for (mode, expected) in [("code", false), ("lib", true)] {
            let cfg = parse(&format!(
                "[[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\n\
                 mode = \"{mode}\"\n",
            ));
            let resolved = resolve(&cfg, &[], Path::new("/x"), &Overrides::default()).unwrap();
            assert_eq!(resolved[0].external_std, expected, "mode {mode}");
        }
    }

    /// Following the mode is a *default*, not a rule: a crate that wants a
    /// self-contained copy of std can still say so.
    #[test]
    fn an_explicit_false_still_inlines_std_in_lib_mode() {
        let cfg = parse(
            "[[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\n\
             mode = \"lib\"\nexternal_std = false\n",
        );
        let resolved = resolve(&cfg, &[], Path::new("/x"), &Overrides::default()).unwrap();
        assert!(!resolved[0].external_std);
    }

    #[test]
    fn the_section_default_applies_to_a_code_target_too() {
        let cfg = parse(
            "[generate]\nexternal_std = true\n\n\
             [[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\n",
        );
        let resolved = resolve(&cfg, &[], Path::new("/x"), &Overrides::default()).unwrap();
        assert!(resolved[0].external_std, "`code`, but asked for explicitly");
    }

    /// Both flag spellings exist because the default is no longer a fixed
    /// `false` - either direction can be the one a run needs to override.
    #[test]
    fn either_flag_spelling_overrides_the_mode_default() {
        let cfg = parse(
            "[[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\n\
             mode = \"lib\"\n",
        );
        let off = Overrides {
            external_std: Some(false),
            ..Default::default()
        };
        let resolved = resolve(&cfg, &[], Path::new("/x"), &off).unwrap();
        assert!(!resolved[0].external_std, "--no-external-std");

        let cfg = parse("[[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\n");
        let on = Overrides {
            external_std: Some(true),
            ..Default::default()
        };
        let resolved = resolve(&cfg, &[], Path::new("/x"), &on).unwrap();
        assert!(resolved[0].external_std, "--external-std");
    }

    #[test]
    fn a_target_may_raise_the_declared_version_but_not_lower_it() {
        let declared = [DeclaredLang {
            language: "rust".into(),
            lang_version: "1.88.0".into(),
        }];
        let raised = parse("[[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.90\"\n");
        let resolved = resolve(&raised, &declared, Path::new("/x"), &Overrides::default()).unwrap();
        assert_eq!(resolved[0].lang_version, "1.90");

        let lowered =
            parse("[[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\n");
        let err = resolve(&lowered, &declared, Path::new("/x"), &Overrides::default()).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("below config.idp's `rust#1.88.0`"),
            "{message}"
        );
        assert!(message.contains("never lower it"), "{message}");
    }

    #[test]
    fn default_framing_absent_is_none() {
        let cfg = parse("[generate]\n");
        let declared = [DeclaredLang {
            language: "rust".into(),
            lang_version: "1.80".into(),
        }];
        let resolved = resolve(&cfg, &declared, Path::new("/x"), &Overrides::default()).unwrap();
        assert_eq!(resolved[0].default_framing, None);
    }

    #[test]
    fn flatten_defaults_to_false_and_is_target_only() {
        let cfg = parse(
            "[[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\n\
             mode = \"lib\"\nflatten = true\n",
        );
        let declared = [];
        let resolved = resolve(&cfg, &declared, Path::new("/x"), &Overrides::default()).unwrap();
        assert!(resolved[0].flatten);

        let cfg = parse(
            "[[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\nmode = \"lib\"\n",
        );
        let resolved = resolve(&cfg, &declared, Path::new("/x"), &Overrides::default()).unwrap();
        assert!(!resolved[0].flatten);
    }

    #[test]
    fn external_std_defaults_to_false() {
        let cfg = parse("[generate]\n");
        let declared = [DeclaredLang {
            language: "rust".into(),
            lang_version: "1.80".into(),
        }];
        let resolved = resolve(&cfg, &declared, Path::new("/x"), &Overrides::default()).unwrap();
        assert!(!resolved[0].external_std);
    }

    #[test]
    fn external_std_section_applies_to_every_target() {
        let cfg = parse("[generate]\nexternal_std = true\n");
        let declared = [DeclaredLang {
            language: "rust".into(),
            lang_version: "1.80".into(),
        }];
        let resolved = resolve(&cfg, &declared, Path::new("/x"), &Overrides::default()).unwrap();
        assert!(resolved[0].external_std);
    }

    #[test]
    fn target_external_std_wins_over_section() {
        let cfg = parse(
            "[generate]\nexternal_std = true\n\n\
             [[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\n\
             external_std = false\n",
        );
        let declared = [];
        let resolved = resolve(&cfg, &declared, Path::new("/x"), &Overrides::default()).unwrap();
        assert!(!resolved[0].external_std);
    }

    #[test]
    fn flag_wins_over_everything_for_external_std() {
        let cfg = parse("[generate]\nexternal_std = false\n");
        let declared = [DeclaredLang {
            language: "rust".into(),
            lang_version: "1.80".into(),
        }];
        let ov = Overrides {
            external_std: Some(true),
            ..Default::default()
        };
        let resolved = resolve(&cfg, &declared, Path::new("/x"), &ov).unwrap();
        assert!(resolved[0].external_std);
    }

    #[test]
    fn validate_on_decode_defaults_to_off() {
        let cfg = parse("[[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\n");
        let resolved = resolve(&cfg, &[], Path::new("/x"), &Overrides::default()).unwrap();
        assert_eq!(resolved[0].validate_on_decode, ValidateOnDecode::Off);
    }

    #[test]
    fn validate_on_decode_target_wins_over_section_and_the_flag_over_both() {
        let cfg = parse(
            "[generate]\nvalidate_on_decode = \"collect\"\n\n\
             [[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\n\
             validate_on_decode = \"first\"\n",
        );
        let resolved = resolve(&cfg, &[], Path::new("/x"), &Overrides::default()).unwrap();
        assert_eq!(resolved[0].validate_on_decode, ValidateOnDecode::First);

        let ov = Overrides {
            validate_on_decode: Some("off"),
            ..Default::default()
        };
        let resolved = resolve(&cfg, &[], Path::new("/x"), &ov).unwrap();
        assert_eq!(resolved[0].validate_on_decode, ValidateOnDecode::Off);
    }

    #[test]
    fn default_framing_is_folded_onto_its_canonical_spelling() {
        let cfg = parse(
            "[generate]\ndefault_framing = \"json-rpc\"\n\n\
             [[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\n",
        );
        let resolved = resolve(&cfg, &[], Path::new("/x"), &Overrides::default()).unwrap();
        assert_eq!(resolved[0].default_framing.as_deref(), Some("jsonrpc"));
    }

    #[test]
    fn an_unknown_default_framing_names_the_accepted_values_and_a_near_miss() {
        let cfg = parse(
            "[generate]\ndefault_framing = \"jsonrcp\"\n\n\
             [[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\n",
        );
        let error = resolve(&cfg, &[], Path::new("/x"), &Overrides::default()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "`default_framing = \"jsonrcp\"` for target `rust` - expected one of `jsonrpc`, \
             `datagram`, `datagram@2` - did you mean `jsonrpc`?"
        );
    }

    #[test]
    fn an_unknown_validate_on_decode_names_the_accepted_values() {
        let cfg = parse(
            "[generate]\nvalidate_on_decode = \"always\"\n\n\
                         [[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.85.0\"\n",
        );
        let error = resolve(&cfg, &[], Path::new("/x"), &Overrides::default()).unwrap_err();
        assert_eq!(
            error.to_string(),
            "`validate_on_decode = \"always\"` for target `rust` - expected one of `off`, `collect`, `first`"
        );
    }
}
