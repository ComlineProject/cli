//! `comline add` — declare a dependency in `config.idp`, resolved and pinned.
//!
//! The dependency is resolved first, the way `comline check` resolves it (a
//! git pin is fetched into `.comline/deps-cache/`, then compiled), so an entry
//! that wouldn't resolve is never written. Its entry then goes into
//! `config.idp` as a text edit, in the file's own indentation, leaving every
//! other line as written, and with `hash` pinned to the content just compiled.

use std::path::{Path, PathBuf};

use comline_core::package::config::dependency::{
    DependencyConfig, DependencySource, DEPS_CACHE_DIR,
};
use comline_core::package::config::idl::grammar as config_grammar;
use comline_core::package::deps;
use comline_core::package::layout::SCHEMAS_DIR;
use miette::{IntoDiagnostic, Result, WrapErr};

use crate::commands::{core_err, ensure_project};
use crate::ui;

/// Where the dependency comes from.
pub enum Source {
    /// A package on disk, relative to this one.
    Path(PathBuf),
    /// A commit of a git repository, and the version it stands for.
    Git {
        uri: String,
        commit: String,
        version: String,
    },
}

impl Source {
    /// From the command line: a `DIR`, or `--git` with its `--commit` and
    /// `--version` (clap enforces which combinations get here).
    pub fn from_args(
        dir: Option<PathBuf>,
        git: Option<String>,
        commit: Option<String>,
        version: Option<String>,
    ) -> Result<Self> {
        match (dir, git, commit, version) {
            (Some(dir), None, None, None) => Ok(Source::Path(dir)),
            (None, Some(uri), Some(commit), Some(version)) => Ok(Source::Git {
                uri,
                commit,
                version,
            }),
            _ => Err(miette::miette!(
                "give either a DIR, or --git with --commit and --version"
            )),
        }
    }
}

pub fn run(work_dir: &Path, name: &str, source: Source, pin: bool) -> Result<()> {
    ensure_project(work_dir)?;
    if !is_identifier(name) {
        return Err(miette::miette!(
            "`{name}` can't name a dependency: it's imported as `use {name}::…`, so it must be \
             an identifier (letters, digits and `_`, not starting with a digit)"
        ));
    }

    let manifest_path = work_dir.join("config.idp");
    let manifest = std::fs::read_to_string(&manifest_path)
        .into_diagnostic()
        .wrap_err("failed to read config.idp")?;
    let declared = declared_dependencies(&manifest)?;
    if declared.iter().any(|d| d.name == name) {
        return Err(miette::miette!(
            "`{name}` is already a dependency in config.idp"
        ));
    }
    // A dependency's schemas merge in under its name, next to the package's own.
    let schemas = work_dir.join(SCHEMAS_DIR);
    if schemas.join(format!("{name}.ids")).exists() || schemas.join(name).is_dir() {
        return Err(miette::miette!(
            "this package already has a `{name}` namespace in {SCHEMAS_DIR}/; pick another name, \
             or `use {name}::…` would mean both"
        ));
    }

    if let Source::Path(dir) = &source {
        if !work_dir.join(dir).join("config.idp").exists() {
            return Err(miette::miette!(
                "`{}` isn't a Comline package (no config.idp there); a dependency's path is \
                 relative to this package's directory",
                dir.display()
            ));
        }
    }

    let mut dependency = DependencyConfig {
        name: name.to_string(),
        source: match &source {
            Source::Path(dir) => DependencySource::Path {
                path: dir.clone(),
                hash: None,
            },
            Source::Git {
                uri,
                commit,
                version,
            } => DependencySource::Git {
                version: version.clone(),
                uri: uri.clone(),
                commit: commit.clone(),
                hash: None,
            },
        },
    };
    // Before fetching anything: the entry has to be writable.
    entry_fields(&dependency)?;

    ui::step(format!("Resolving {name}{}", ui::at_path(work_dir)));
    let spinner = ui::spinner(match source {
        Source::Path(_) => "compiling",
        Source::Git { .. } => "fetching and compiling",
    });
    let cache_dir = work_dir.join(DEPS_CACHE_DIR);
    let resolve = || deps::resolve(&dependency, work_dir, &cache_dir);
    // `resolve` warns that an entry without a hash is trusted and suggests
    // the very pin this command writes next: keep that out of a normal run.
    let resolved = if ui::verbose() {
        resolve()
    } else {
        tracing::subscriber::with_default(tracing::subscriber::NoSubscriber::default(), resolve)
    };
    ui::finish_spinner(spinner);
    let resolved = resolved.map_err(|e| core_err("couldn't resolve the dependency", e))?;

    let hash = pin.then(|| format!("blake3:{}", resolved.content_hash.to_hex()));
    match &mut dependency.source {
        DependencySource::Path { hash: h, .. } | DependencySource::Git { hash: h, .. } => {
            *h = hash.clone()
        }
        DependencySource::Registry { .. } => {
            unreachable!("`comline add` never builds a registry source")
        }
    }

    let fields = entry_fields(&dependency)?;
    let updated = with_dependency(&manifest, name, &fields);
    verify(&updated, &declared, &dependency).map_err(|problem| {
        miette::miette!(
            "couldn't add the entry to config.idp safely ({problem}); add it by hand:\n{}",
            entry_lines(name, &fields, "    ", "    ").join("\n")
        )
    })?;
    std::fs::write(&manifest_path, updated)
        .into_diagnostic()
        .wrap_err("failed to write config.idp")?;

    ui::success(format!("Added {name} to config.idp"));
    match &hash {
        Some(hash) => ui::detail(format!("pinned: hash = \"{hash}\"")),
        None => ui::note("not pinned (--no-hash): a change to it won't fail the build"),
    }
    if let Source::Git { .. } = source {
        ui::detail(format!("fetched into {}", resolved.resolved_path.display()));
    }
    ui::detail(format!("import its schemas as `use {name}::…`"));
    Ok(())
}

fn declared_dependencies(manifest: &str) -> Result<Vec<DependencyConfig>> {
    let congregation = config_grammar::parse(manifest).map_err(|_| {
        miette::miette!("config.idp doesn't parse; fix it before adding a dependency")
    })?;
    let declared = DependencyConfig::parse_dependencies(&congregation.assignments)
        .map_err(|e| miette::miette!("config.idp: {e}"))?;
    Ok(declared.into_values().collect())
}

/// The entry's `key = "value"` lines, in the order the docs write them.
fn entry_fields(dependency: &DependencyConfig) -> Result<Vec<(&'static str, String)>> {
    let mut fields = match &dependency.source {
        // `/` works as a separator everywhere, and a `\` would need escaping
        // that `config.idp` strings don't undo.
        DependencySource::Path { path, .. } => {
            vec![("path", path.to_string_lossy().replace('\\', "/"))]
        }
        DependencySource::Git {
            version,
            uri,
            commit,
            ..
        } => {
            vec![
                ("version", version.clone()),
                ("uri", uri.clone()),
                ("commit", commit.clone()),
            ]
        }
        DependencySource::Registry { .. } => {
            unreachable!("`comline add` never builds a registry source")
        }
    };
    if let Some(hash) = dependency.declared_hash() {
        fields.push(("hash", hash.to_string()));
    }
    if let Some((key, _)) = fields.iter().find(|(_, value)| value.contains(['"', '\\'])) {
        return Err(miette::miette!(
            "the dependency's `{key}` can't contain `\"` or `\\`"
        ));
    }
    Ok(fields)
}

/// `manifest` with the entry `name = { fields }` added to its top-level
/// `dependencies` block, or in a new block at the end. Every other line is
/// left as written; the entry follows the block's own indentation.
fn with_dependency(manifest: &str, name: &str, fields: &[(&str, String)]) -> String {
    let Some((open, close)) = dependencies_block(manifest) else {
        let entry = entry_lines(name, fields, "    ", "    ").join("\n");
        return format!(
            "{}\n\ndependencies = {{\n{entry}\n}}\n",
            manifest.trim_end()
        );
    };

    let block_indent = indent_of(manifest, open);
    let inner = &manifest[open + 1..close];
    // The first entry's line sets the indentation; an empty block gets one
    // level more than its own line.
    let first_entry = inner
        .find(|c: char| !c.is_whitespace())
        .map(|i| open + 1 + i);
    let entry_indent = match first_entry {
        Some(at) if manifest[open..at].contains('\n') => indent_of(manifest, at).to_string(),
        _ => format!("{block_indent}{}", unit(block_indent)),
    };
    let step = entry_indent
        .strip_prefix(block_indent)
        .filter(|s| !s.is_empty())
        .unwrap_or(unit(block_indent));
    let mut entry = entry_lines(name, fields, &entry_indent, step).join("\n");
    // Entries already separated by blank lines get one before this one too.
    if first_entry.is_some() && has_blank_line(inner.trim()) {
        entry.insert(0, '\n');
    }

    let line_start = manifest[..close].rfind('\n').map_or(0, |i| i + 1);
    if manifest[line_start..close].trim().is_empty() {
        // `}` on its own line: the entry goes on the lines before it.
        format!(
            "{}{entry}\n{}",
            &manifest[..line_start],
            &manifest[line_start..]
        )
    } else {
        // `}` after other text (`dependencies = {}`): break the line.
        format!(
            "{}\n{entry}\n{block_indent}{}",
            manifest[..close].trim_end(),
            &manifest[close..]
        )
    }
}

fn entry_lines(name: &str, fields: &[(&str, String)], indent: &str, step: &str) -> Vec<String> {
    let mut lines = vec![format!("{indent}{name} = {{")];
    lines.extend(
        fields
            .iter()
            .map(|(key, value)| format!("{indent}{step}{key} = \"{value}\"")),
    );
    lines.push(format!("{indent}}}"));
    lines
}

/// Byte offsets of the `{` and matching `}` of the top-level
/// `dependencies = { ... }`, skipping strings and comments.
fn dependencies_block(manifest: &str) -> Option<(usize, usize)> {
    let tokens = tokens(manifest);
    let mut depth = 0usize;
    for (i, &(token, _)) in tokens.iter().enumerate() {
        match token {
            Token::Open => depth += 1,
            Token::Close => depth = depth.saturating_sub(1),
            Token::Word("dependencies") if depth == 0 => {
                if let [(Token::Equals, _), (Token::Open, open), ..] = tokens[i + 1..] {
                    let mut inner_depth = 0usize;
                    for &(token, at) in &tokens[i + 2..] {
                        match token {
                            Token::Open => inner_depth += 1,
                            Token::Close if inner_depth == 1 => return Some((open, at)),
                            Token::Close => inner_depth -= 1,
                            _ => {}
                        }
                    }
                    return None;
                }
            }
            _ => {}
        }
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Token<'a> {
    Word(&'a str),
    Equals,
    /// `{` or `[`.
    Open,
    /// `}` or `]`.
    Close,
    Other,
}

/// `config.idp`'s tokens with their byte offsets, minus whitespace, comments
/// (`//`, `/* */`) and string contents.
fn tokens(text: &str) -> Vec<(Token<'_>, usize)> {
    let bytes = text.as_bytes();
    let is_word =
        |b: u8| b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'#' | b'@' | b'.');
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                i = text[i..].find('\n').map_or(bytes.len(), |n| i + n);
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i = text[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |n| i + 2 + n + 2);
            }
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                tokens.push((Token::Other, i));
                i += 1;
            }
            b'{' | b'[' => {
                tokens.push((Token::Open, i));
                i += 1;
            }
            b'}' | b']' => {
                tokens.push((Token::Close, i));
                i += 1;
            }
            b'=' => {
                tokens.push((Token::Equals, i));
                i += 1;
            }
            _ if is_word(b) => {
                let start = i;
                while i < bytes.len() && is_word(bytes[i]) {
                    i += 1;
                }
                tokens.push((Token::Word(&text[start..i]), start));
            }
            _ if b.is_ascii_whitespace() => i += 1,
            _ => {
                tokens.push((Token::Other, i));
                i += 1;
            }
        }
    }
    tokens
}

/// The leading whitespace of the line `at` is on.
fn indent_of(text: &str, at: usize) -> &str {
    let start = text[..at].rfind('\n').map_or(0, |i| i + 1);
    let line = &text[start..];
    &line[..line.len() - line.trim_start_matches([' ', '\t']).len()]
}

/// One indentation level, in the file's own style: a tab if the line is
/// tab-indented, else four spaces.
fn unit(indent: &str) -> &'static str {
    if indent.starts_with('\t') {
        "\t"
    } else {
        "    "
    }
}

fn has_blank_line(text: &str) -> bool {
    text.lines().any(|line| line.trim().is_empty())
}

/// Whether `updated` declares exactly what was declared before plus
/// `dependency`, as `comline check` will read it.
fn verify(
    updated: &str,
    before: &[DependencyConfig],
    dependency: &DependencyConfig,
) -> Result<(), String> {
    let after = declared_dependencies(updated).map_err(|e| e.to_string())?;
    let Some(added) = after.iter().find(|d| d.name == dependency.name) else {
        return Err("the new entry isn't read back".to_string());
    };
    if after.len() != before.len() + 1 {
        return Err("other entries changed".to_string());
    }
    let same = match (&added.source, &dependency.source) {
        (DependencySource::Path { path: a, .. }, DependencySource::Path { path: b, .. }) => {
            a.to_string_lossy().replace('\\', "/") == b.to_string_lossy().replace('\\', "/")
        }
        (
            DependencySource::Git {
                version: va,
                uri: ua,
                commit: ca,
                ..
            },
            DependencySource::Git {
                version: vb,
                uri: ub,
                commit: cb,
                ..
            },
        ) => (va, ua, ca) == (vb, ub, cb),
        _ => false,
    };
    match same && added.declared_hash() == dependency.declared_hash() {
        true => Ok(()),
        false => Err("the entry reads back differently".to_string()),
    }
}

fn is_identifier(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fields() -> Vec<(&'static str, String)> {
        vec![
            ("path", "../shared".to_string()),
            ("hash", "blake3:abc".to_string()),
        ]
    }

    const HEAD: &str = "congregation app\nspecification_version = 1\n";

    #[test]
    fn a_manifest_without_dependencies_gets_a_block_at_the_end() {
        let manifest = format!("{HEAD}\ncode_generation = {{\n    languages = {{\n        rust#1.70.0 = {{}}\n    }}\n}}\n");
        assert_eq!(
            with_dependency(&manifest, "shared", &fields()),
            format!(
                "{}\n\ndependencies = {{\n    shared = {{\n        path = \"../shared\"\n        hash = \"blake3:abc\"\n    }}\n}}\n",
                manifest.trim_end()
            )
        );
    }

    #[test]
    fn an_entry_goes_last_in_the_block_in_its_indentation() {
        let manifest = format!(
            "{HEAD}\ndependencies = {{\n  net = {{\n    version = \"1.0.0\"\n    uri = \"x\"\n    commit = \"c\"\n  }}\n}}\n\n// after\n"
        );
        let updated = with_dependency(&manifest, "shared", &fields());
        assert_eq!(
            updated,
            format!(
                "{HEAD}\ndependencies = {{\n  net = {{\n    version = \"1.0.0\"\n    uri = \"x\"\n    commit = \"c\"\n  }}\n  shared = {{\n    path = \"../shared\"\n    hash = \"blake3:abc\"\n  }}\n}}\n\n// after\n"
            )
        );
    }

    #[test]
    fn blank_lines_between_entries_are_kept_up() {
        let manifest = format!(
            "{HEAD}dependencies = {{\n    a = {{\n        path = \"../a\"\n    }}\n\n    b = {{\n        path = \"../b\"\n    }}\n}}\n"
        );
        let updated = with_dependency(&manifest, "c", &[("path", "../c".to_string())]);
        assert!(
            updated.ends_with("    }\n\n    c = {\n        path = \"../c\"\n    }\n}\n"),
            "{updated}"
        );
    }

    #[test]
    fn an_empty_block_and_tabs() {
        let manifest = format!("{HEAD}dependencies = {{}}\n");
        assert_eq!(
            with_dependency(&manifest, "a", &[("path", "../a".to_string())]),
            format!("{HEAD}dependencies = {{\n    a = {{\n        path = \"../a\"\n    }}\n}}\n")
        );

        let manifest =
            format!("{HEAD}dependencies = {{\n\ta = {{\n\t\tpath = \"../a\"\n\t}}\n}}\n");
        let updated = with_dependency(&manifest, "b", &[("path", "../b".to_string())]);
        assert!(
            updated.ends_with("\t}\n\tb = {\n\t\tpath = \"../b\"\n\t}\n}\n"),
            "{updated:?}"
        );
    }

    #[test]
    fn braces_in_strings_and_comments_and_nested_keys_are_skipped() {
        let manifest = format!(
            "{HEAD}// dependencies = {{ not = this }}\nnotes = \"dependencies = {{\"\n/* }} */\nmeta = {{\n    dependencies = {{}}\n}}\n\ndependencies = {{\n    a = {{\n        path = \"../a\" // a }}\n    }}\n}}\n"
        );
        let (open, close) = dependencies_block(&manifest).unwrap();
        assert_eq!(
            &manifest[open..=close],
            "{\n    a = {\n        path = \"../a\" // a }\n    }\n}"
        );
    }

    #[test]
    fn every_edit_reads_back_with_cores_parser() {
        for manifest in [
            HEAD.to_string(),
            format!("{HEAD}dependencies = {{}}\n"),
            format!("{HEAD}\ndependencies = {{\n    net = {{\n        version = \"1.0.0\"\n        uri = \"x\"\n        commit = \"c\"\n    }}\n}}\n"),
        ] {
            let before = declared_dependencies(&manifest).unwrap();
            let dependency = DependencyConfig {
                name: "shared".to_string(),
                source: DependencySource::Path { path: "../shared".into(), hash: Some("blake3:abc".to_string()) },
            };
            let updated = with_dependency(&manifest, "shared", &entry_fields(&dependency).unwrap());
            assert_eq!(verify(&updated, &before, &dependency), Ok(()), "{updated}");
        }
    }

    #[test]
    fn names_must_be_identifiers() {
        assert!(is_identifier("shared_types"));
        assert!(is_identifier("_x1"));
        for bad in ["shared-types", "1st", "", "a::b", "naïve"] {
            assert!(!is_identifier(bad), "{bad}");
        }
    }
}
