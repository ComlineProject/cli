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

use comline_core::utils::templating::recurse_render;
use miette::{miette, IntoDiagnostic, Result, WrapErr};
use serde::{Deserialize, Serialize};

/// File looked for in the project directory.
pub const FILE_NAME: &str = "comline.toml";

/// Default output root, relative to `comline.toml`.
pub const DEFAULT_OUT: &str = "generated";
/// Default on-disk layout under a target's root.
pub const DEFAULT_LAYOUT: &str = "{{language}}/{{namespace}}.{{ext}}";
/// Default emit form. Only `"code"` is implemented today.
pub const DEFAULT_MODE: &str = "code";

/// Which package versions to generate bindings for.
///
/// TOML: `package_versions = "latest"` (default) | `"all"` | `"0.3.0"` |
/// `["0.3.0", "0.4.0"]`. `latest` is the working tree; anything else reads
/// committed versions from the CAS chain.
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
    /// Output root shared by every target unless overridden.
    pub out: Option<String>,
    /// Layout template shared by every target unless overridden.
    pub layout: Option<String>,
    /// Emit form shared by every target unless overridden.
    pub mode: Option<String>,
    /// Version selection shared by every target unless overridden.
    pub package_versions: Option<VersionSpec>,
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
    /// Language/toolchain version selecting the generator (e.g. `1.70.0`).
    /// Falls back to the version the congregation declared for this language.
    pub lang_version: Option<String>,
    pub out: Option<String>,
    pub layout: Option<String>,
    pub mode: Option<String>,
    pub package_versions: Option<VersionSpec>,
}

/// A non-empty `COMLINE_GENERATE_*` env var, if set.
fn env_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|s| !s.is_empty())
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
/// The base list is `[[generate.target]]` if present, otherwise one target per
/// declared language. `--target` filters it (case-insensitive). The `--out` /
/// `--layout` / `--mode` flags apply to the single remaining target, or to the
/// `--target`-named one; using them with several targets and no `--target` is an
/// error. `COMLINE_GENERATE_{OUT,LAYOUT,MODE}` sit just below the flags but apply
/// to every target regardless (a deliberate global for CI).
pub fn resolve(
    cfg: &ComlineToml,
    declared: &[DeclaredLang],
    work_dir: &Path,
    ov: &Overrides,
) -> Result<Vec<ResolvedTarget>> {
    let base: Vec<Target> = if cfg.generate.targets.is_empty() {
        declared
            .iter()
            .map(|d| Target {
                language: d.language.clone(),
                lang_version: Some(d.lang_version.clone()),
                out: None,
                layout: None,
                mode: None,
                package_versions: None,
            })
            .collect()
    } else {
        cfg.generate.targets.clone()
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
            None => Err(miette!(
                "nothing to generate: add `[[generate.target]]` to {} \
                 or `code_generation.languages` to config.idp",
                FILE_NAME
            )),
        };
    }

    let field_override = ov.out.is_some() || ov.layout.is_some() || ov.mode.is_some();
    if field_override && selected.len() > 1 && ov.target.is_none() {
        return Err(miette!(
            "`--out` / `--layout` / `--mode` need `--target <lang>` when more \
             than one target is configured ({} here)",
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

        let lang_version = t
            .lang_version
            .clone()
            .or_else(|| {
                declared
                    .iter()
                    .find(|d| d.language.eq_ignore_ascii_case(&t.language))
                    .map(|d| d.lang_version.clone())
            })
            .ok_or_else(|| {
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

        resolved.push(ResolvedTarget {
            language: t.language.clone(),
            lang_version,
            out: work_dir.join(out),
            layout,
            mode,
            versions,
        });
    }

    Ok(resolved)
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
             [[generate.target]]\nlanguage = \"rust\"\nlang_version = \"1.70.0\"\n\
             package_versions = \"latest\"\n",
        );
        let declared = [];
        let resolved = resolve(&cfg, &declared, Path::new("/x"), &Overrides::default()).unwrap();
        assert_eq!(resolved[0].versions, VersionSpec::Latest);
    }
}
