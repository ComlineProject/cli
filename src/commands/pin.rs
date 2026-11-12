//! `comline deps pin` — pin a dependency's `hash` to the content it has now.
//!
//! For a dependency whose content changed: the build fails on the old pin
//! until it's updated. The dependency is resolved first, the way `comline
//! deps add` resolves it, so a dependency that doesn't resolve is never
//! pinned. Only its entry in `config.idp` changes; every other line is left
//! as written.

use std::path::Path;

use comline_core::package::config::dependency::{
    DependencyConfig, DependencySource, DEPS_CACHE_DIR,
};
use comline_core::package::deps;
use miette::{IntoDiagnostic, Result, WrapErr};

use crate::commands::accept::{self, entry, with_acceptance, with_line};
use crate::commands::add::{declared_dependencies, declared_named, dependencies_block};
use crate::commands::{core_err, ensure_project};
use crate::ui;

/// Another commit of a git dependency, and the version it stands for.
pub struct Move {
    pub commit: String,
    pub version: String,
}

pub fn run(work_dir: &Path, name: &str, to: Option<Move>, accept_unverified: bool) -> Result<()> {
    ensure_project(work_dir)?;

    let manifest_path = work_dir.join("config.idp");
    let manifest = std::fs::read_to_string(&manifest_path)
        .into_diagnostic()
        .wrap_err("failed to read config.idp")?;
    let declared = declared_dependencies(&manifest)?;
    let current = declared_named(&declared, name)?;
    let old_hash = current.declared_hash().map(str::to_string);

    // What the entry should read once pinned, resolved before anything is written.
    let mut dependency = current.clone();
    match (&mut dependency.source, &to) {
        (
            DependencySource::Git {
                commit, version, ..
            },
            Some(to),
        ) => {
            *commit = to.commit.clone();
            *version = to.version.clone();
        }
        (_, Some(_)) => {
            return Err(miette::miette!(
                "`{name}` isn't a git dependency, so there's no commit to move: --commit and \
                 --version are for `--git` dependencies"
            ))
        }
        _ => {}
    }
    for value in [commit_of(&dependency), version_of(&dependency)]
        .into_iter()
        .flatten()
    {
        if value.contains(['"', '\\']) {
            return Err(miette::miette!(
                "a commit or version can't contain `\"` or `\\`"
            ));
        }
    }
    set_hash(&mut dependency, None);

    ui::step(format!("Resolving {name}{}", ui::at_path(work_dir)));
    let spinner = ui::spinner(match dependency.source {
        DependencySource::Git { .. } => "fetching and compiling",
        _ => "compiling",
    });
    let cache_dir = work_dir.join(DEPS_CACHE_DIR);
    // Resolving without a hash warns that it's trusted, and suggests the very
    // pin this command writes: keep that out of a normal run.
    let resolve = |dependency: &DependencyConfig| {
        accept::quietly(|| deps::resolve(dependency, work_dir, &cache_dir))
    };
    let mut resolved = resolve(&dependency);
    ui::finish_spinner(spinner);

    // Content that gives up more than the entry accepts resolves only once
    // that's accepted: show what, and ask, as `add` does.
    if resolved.is_err() {
        let mut wanted = match dependency.package_dir(work_dir) {
            Some(dir) if dir.join("config.idp").exists() => {
                accept::unverified_in(&dir).unwrap_or_default()
            }
            _ => vec![],
        };
        wanted.retain(|p| dependency.accept_unverified.get(&p.protocol) != Some(&p.set));
        if !wanted.is_empty() {
            if !accept::decide(name, &wanted, accept_unverified, "--accept-unverified")? {
                ui::note(format!(
                    "Not accepted: {name} wasn't pinned, config.idp is unchanged"
                ));
                return Ok(());
            }
            for p in wanted {
                dependency.accept_unverified.insert(p.protocol, p.set);
            }
            resolved = resolve(&dependency);
        }
    }
    let resolved = resolved.map_err(|e| core_err("couldn't resolve the dependency", e))?;
    let new_hash = format!("blake3:{}", resolved.content_hash.to_hex());
    set_hash(&mut dependency, Some(new_hash.clone()));

    if old_hash.as_deref() == Some(new_hash.as_str())
        && to.is_none()
        && dependency.accept_unverified == current.accept_unverified
    {
        ui::success(format!(
            "{name} is already pinned to that content: config.idp is unchanged"
        ));
        ui::detail(format!("hash = \"{new_hash}\""));
        return Ok(());
    }

    let updated = with_pin(&manifest, current, &dependency)
        .and_then(|updated| verify(&updated, &declared, &dependency).map(|()| updated))
        .map_err(|problem| {
            miette::miette!(
                "couldn't write the pin into config.idp safely ({problem}); set it in `{name}`'s \
                 entry by hand:\n    hash = \"{new_hash}\""
            )
        })?;
    std::fs::write(&manifest_path, updated)
        .into_diagnostic()
        .wrap_err("failed to write config.idp")?;

    ui::success(format!("Pinned {name} in config.idp"));
    ui::detail(format!(
        "was: {}",
        old_hash.as_deref().unwrap_or("not pinned")
    ));
    ui::detail(format!("now: {new_hash}"));
    if let Some(to) = &to {
        ui::detail(format!("moved to {} ({})", to.commit, to.version));
    }
    if dependency.accept_unverified != current.accept_unverified {
        ui::detail("accept_unverified updated");
    }
    Ok(())
}

fn commit_of(dependency: &DependencyConfig) -> Option<&str> {
    match &dependency.source {
        DependencySource::Git { commit, .. } => Some(commit),
        _ => None,
    }
}

fn version_of(dependency: &DependencyConfig) -> Option<&str> {
    match &dependency.source {
        DependencySource::Git { version, .. } => Some(version),
        _ => None,
    }
}

fn set_hash(dependency: &mut DependencyConfig, value: Option<String>) {
    match &mut dependency.source {
        DependencySource::Path { hash, .. }
        | DependencySource::Git { hash, .. }
        | DependencySource::Registry { hash, .. } => *hash = value,
    }
}

/// `manifest` with `after`'s `hash` (and, if moved, `commit` and `version`;
/// and `accept_unverified`, if it changed) written into `before`'s entry.
fn with_pin(
    manifest: &str,
    before: &DependencyConfig,
    after: &DependencyConfig,
) -> Result<String, String> {
    let mut text = manifest.to_string();
    if let (
        DependencySource::Git {
            commit: c0,
            version: v0,
            ..
        },
        DependencySource::Git {
            commit: c1,
            version: v1,
            ..
        },
    ) = (&before.source, &after.source)
    {
        if c0 != c1 {
            text = with_field(&text, &after.name, "commit", c1)?;
        }
        if v0 != v1 {
            text = with_field(&text, &after.name, "version", v1)?;
        }
    }
    if let Some(hash) = after.declared_hash() {
        text = with_field(&text, &after.name, "hash", hash)?;
    }
    if after.accept_unverified != before.accept_unverified {
        text = with_acceptance(&text, &after.name, &after.accept_unverified)?;
    }
    Ok(text)
}

/// `manifest` with `key = "value"` in `name`'s entry: the string replaced
/// where the entry has the key, else a line added last.
fn with_field(manifest: &str, name: &str, key: &str, value: &str) -> Result<String, String> {
    let (block_open, block_close) =
        dependencies_block(manifest).ok_or("no `dependencies` block")?;
    let (open, close) = entry(manifest, block_open, block_close, name)
        .ok_or_else(|| format!("no `{name} = {{ ... }}` entry in it"))?;

    match string_value(manifest, open, close, key)? {
        Some((start, end)) => Ok(format!(
            "{}\"{value}\"{}",
            &manifest[..start],
            &manifest[end..]
        )),
        None => Ok(with_line(manifest, close, &format!("{key} = \"{value}\""))),
    }
}

/// The span, quotes included, of the string in a `key = "..."` directly
/// inside an entry; `None` if it has no such key.
fn string_value(
    manifest: &str,
    open: usize,
    close: usize,
    key: &str,
) -> Result<Option<(usize, usize)>, String> {
    use crate::commands::add::{tokens, Token};

    let toks: Vec<_> = tokens(manifest)
        .into_iter()
        .filter(|&(_, at)| at > open && at < close)
        .collect();
    let mut depth = 0usize;
    for (i, &(token, _)) in toks.iter().enumerate() {
        match token {
            Token::Open => depth += 1,
            Token::Close => depth = depth.saturating_sub(1),
            Token::Word(word) if depth == 0 && word == key => {
                // A string's token sits at its closing quote.
                if let [(Token::Equals, equals), (Token::Other, end), ..] = toks[i + 1..] {
                    let rest = &manifest[equals + 1..];
                    let start = equals + 1 + rest.len() - rest.trim_start().len();
                    if manifest.as_bytes().get(start) == Some(&b'"')
                        && manifest.as_bytes().get(end) == Some(&b'"')
                    {
                        return Ok(Some((start, end + 1)));
                    }
                }
                return Err(format!("its `{key}` isn't a plain string"));
            }
            _ => {}
        }
    }
    Ok(None)
}

/// Whether `updated` declares what was declared before, with only
/// `dependency`'s entry changed to what it now is.
fn verify(
    updated: &str,
    before: &[DependencyConfig],
    dependency: &DependencyConfig,
) -> Result<(), String> {
    let after = declared_dependencies(updated).map_err(|e| e.to_string())?;
    if after.len() != before.len() {
        return Err("other entries changed".to_string());
    }
    for old in before {
        let Some(new) = after.iter().find(|d| d.name == old.name) else {
            return Err(format!("`{}` isn't read back", old.name));
        };
        let expected = if old.name == dependency.name {
            dependency
        } else {
            old
        };
        if new.accept_unverified != expected.accept_unverified
            || format!("{:?}", new.source) != format!("{:?}", expected.source)
        {
            return Err(format!("`{}` reads back differently", old.name));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEAD: &str = "congregation app\nspecification_version = 1\n";

    #[test]
    fn a_hash_is_replaced_where_it_stands() {
        let manifest = format!(
            "{HEAD}dependencies = {{\n    a = {{\n        path = \"../a\"\n        hash = \"blake3:old\" // pin\n        accept_unverified = {{ A = [\"schema_agreement\"] }}\n    }}\n    b = {{\n        hash = \"blake3:b\"\n        path = \"../b\"\n    }}\n}}\n"
        );
        assert_eq!(
            with_field(&manifest, "a", "hash", "blake3:new").unwrap(),
            manifest.replace("blake3:old", "blake3:new")
        );
        assert_eq!(
            with_field(&manifest, "b", "hash", "blake3:new").unwrap(),
            manifest.replace("blake3:b", "blake3:new")
        );
    }

    #[test]
    fn a_missing_hash_is_added_last_in_the_entrys_style() {
        let manifest = format!(
            "{HEAD}dependencies = {{\n  a = {{\n    path = \"../a\"\n  }}\n  b = {{ path = \"../b\" }}\n}}\n"
        );
        assert_eq!(
            with_field(&manifest, "a", "hash", "blake3:x").unwrap(),
            format!(
                "{HEAD}dependencies = {{\n  a = {{\n    path = \"../a\"\n    hash = \"blake3:x\"\n  }}\n  b = {{ path = \"../b\" }}\n}}\n"
            )
        );
        assert_eq!(
            with_field(&manifest, "b", "hash", "blake3:x").unwrap(),
            format!(
                "{HEAD}dependencies = {{\n  a = {{\n    path = \"../a\"\n  }}\n  b = {{ path = \"../b\" hash = \"blake3:x\" }}\n}}\n"
            )
        );
    }

    #[test]
    fn a_key_nested_deeper_isnt_the_entrys() {
        let manifest = format!(
            "{HEAD}dependencies = {{\n    a = {{\n        path = \"../a\"\n        accept_unverified = {{ hash = [\"schema_agreement\"] }}\n    }}\n}}\n"
        );
        let updated = with_field(&manifest, "a", "hash", "blake3:x").unwrap();
        assert!(
            updated.ends_with("] }\n        hash = \"blake3:x\"\n    }\n}\n"),
            "{updated}"
        );
    }
}
