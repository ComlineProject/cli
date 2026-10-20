//! `comline accept` — accept what a dependency's `@unverified` protocols give
//! up, by writing `accept_unverified` into its `config.idp` entry.
//!
//! Accepting is a choice the consumer writes down, so it's never silent: what
//! each protocol gives up, and why, is shown first, and on a terminal the
//! user is asked (default no). Without a terminal there's nobody to ask, and
//! it takes a flag. `comline add` uses the same steps for a new dependency.

use std::collections::BTreeMap;
use std::io::{self, IsTerminal, Write};
use std::path::Path;

use comline_core::package::config::dependency::{DependencyConfig, DEPS_CACHE_DIR};
use comline_core::package::deps;
use comline_core::schema::ir::frozen::unit::Guarantee;
use comline_core::schema::ir::unverified;
use miette::{IntoDiagnostic, Result, WrapErr};

use crate::commands::add::{
    declared_dependencies, dependencies_block, indent_of, tokens, unit, Token,
};
use crate::commands::{core_err, ensure_project};
use crate::error::CliError;
use crate::ui;

/// One `@unverified` protocol in a dependency, as its own build froze it.
#[derive(Debug, Clone, PartialEq)]
pub struct Unverified {
    pub protocol: String,
    pub set: Vec<Guarantee>,
    pub reason: Option<String>,
}

/// What a dependency's package gives up: every `@unverified` protocol in it,
/// compiled as its own package (with its own settings and defaults).
pub fn unverified_in(package_dir: &Path) -> Result<Vec<Unverified>> {
    let compile = || comline_core::package::build::compile_package(package_dir);
    let context = quietly(compile).map_err(|e| core_err("couldn't compile the dependency", e))?;
    let mut found = Vec::new();
    for schema in &context.schema_contexts {
        let schema = schema.borrow();
        let frozen = schema.frozen_schema.borrow();
        let Some(units) = frozen.as_ref() else {
            continue;
        };
        for (protocol, set, reason) in unverified::protocols(units) {
            found.push(Unverified {
                protocol: protocol.to_string(),
                set: set.to_vec(),
                reason: reason.map(str::to_string),
            });
        }
    }
    Ok(found)
}

/// Run `f` with core's tracing warnings (an unpinned dependency, say) kept
/// out of a normal run.
pub fn quietly<T>(f: impl FnOnce() -> T) -> T {
    if ui::verbose() {
        f()
    } else {
        tracing::subscriber::with_default(tracing::subscriber::NoSubscriber::default(), f)
    }
}

/// Show what `dependency` gives up, then decide: `yes` accepts; otherwise a
/// terminal is asked (default no), and anything else is refused with
/// [`CliError::AcceptanceRequired`]. `Ok(false)` when the user declines.
pub fn decide(
    dependency: &str,
    protocols: &[Unverified],
    yes: bool,
    flag: &'static str,
) -> Result<bool> {
    ui::warn(format!(
        "{dependency} has {} unverified protocol{}:",
        protocols.len(),
        if protocols.len() == 1 { "" } else { "s" }
    ));
    for p in protocols {
        let reason = p
            .reason
            .as_deref()
            .map(|r| format!(": \"{r}\""))
            .unwrap_or_default();
        ui::detail(format!(
            "{} gives up {}{reason}",
            p.protocol,
            unverified::describe(&p.set)
        ));
    }
    if yes {
        return Ok(true);
    }
    if !io::stdin().is_terminal() {
        return Err(CliError::AcceptanceRequired { flag }.into());
    }
    let names: Vec<&str> = protocols.iter().map(|p| p.protocol.as_str()).collect();
    print!("Accept {}? [y/N] ", names.join(", "));
    io::stdout().flush().into_diagnostic()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line).into_diagnostic()?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// `{ A = ["schema_agreement"], B = [...] }`, as `config.idp` reads it.
pub fn render(accepted: &BTreeMap<String, Vec<Guarantee>>) -> String {
    let entries: Vec<String> = accepted
        .iter()
        .map(|(protocol, set)| format!("{protocol} = {}", unverified::written(set)))
        .collect();
    format!("{{ {} }}", entries.join(" "))
}

/// `comline accept DEPENDENCY[::PROTOCOL]`.
pub fn run(work_dir: &Path, target: &str, yes: bool) -> Result<()> {
    ensure_project(work_dir)?;
    let (name, only) = match target.split_once("::") {
        Some((name, protocol)) => (name, Some(protocol)),
        None => (target, None),
    };

    let manifest_path = work_dir.join("config.idp");
    let manifest = std::fs::read_to_string(&manifest_path)
        .into_diagnostic()
        .wrap_err("failed to read config.idp")?;
    let declared = declared_dependencies(&manifest)?;
    let Some(dependency) = declared.iter().find(|d| d.name == name) else {
        return Err(miette::miette!("`{name}` isn't a dependency in config.idp"));
    };
    let Some(dir) = dependency
        .package_dir(work_dir)
        .filter(|d| d.join("config.idp").exists())
    else {
        return Err(miette::miette!(
            "`{name}` isn't fetched yet; run `comline check` to fetch it first"
        ));
    };

    ui::step(format!("Checking {name}{}", ui::at_path(work_dir)));
    let mut wanted = unverified_in(&dir)?;
    if let Some(protocol) = only {
        wanted.retain(|p| p.protocol == protocol);
        if wanted.is_empty() {
            return Err(miette::miette!(
                "`{name}` has no unverified protocol `{protocol}`"
            ));
        }
    }
    wanted.retain(|p| dependency.accept_unverified.get(&p.protocol) != Some(&p.set));
    if wanted.is_empty() {
        ui::success(format!(
            "Nothing to accept: {target} is already accepted as it stands"
        ));
        return Ok(());
    }

    if !decide(name, &wanted, yes, "--yes")? {
        ui::note("Not accepted: config.idp is unchanged");
        return Ok(());
    }

    let mut accepted = dependency.accept_unverified.clone();
    for p in &wanted {
        accepted.insert(p.protocol.clone(), p.set.clone());
    }
    let updated = with_acceptance(&manifest, name, &accepted)
        .and_then(|updated| verify(&updated, &declared, name, &accepted).map(|()| updated))
        .map_err(|problem| {
            miette::miette!(
                "couldn't write the acceptance into config.idp safely ({problem}); add it to \
                 `{name}`'s entry by hand:\n    accept_unverified = {}",
                render(&accepted)
            )
        })?;

    // The edit reads back; check it now resolves as the build will.
    let mut check = dependency.clone();
    check.accept_unverified = accepted.clone();
    let cache_dir = work_dir.join(DEPS_CACHE_DIR);
    quietly(|| deps::resolve(&check, work_dir, &cache_dir))
        .map_err(|e| core_err("the dependency still doesn't resolve", e))?;

    std::fs::write(&manifest_path, updated)
        .into_diagnostic()
        .wrap_err("failed to write config.idp")?;
    let names: Vec<&str> = wanted.iter().map(|p| p.protocol.as_str()).collect();
    ui::success(format!(
        "Accepted {} from {name} in config.idp",
        names.join(", ")
    ));
    Ok(())
}

/// `manifest` with `name`'s entry carrying `accept_unverified = accepted`,
/// replacing one already there. Every other byte is left as written.
pub fn with_acceptance(
    manifest: &str,
    name: &str,
    accepted: &BTreeMap<String, Vec<Guarantee>>,
) -> Result<String, String> {
    let (block_open, block_close) =
        dependencies_block(manifest).ok_or("no `dependencies` block")?;
    let (entry_open, entry_close) = entry(manifest, block_open, block_close, name)
        .ok_or_else(|| format!("no `{name} = {{ ... }}` entry in it"))?;
    let line = format!("accept_unverified = {}", render(accepted));

    if let Some((start, end)) = existing(manifest, entry_open, entry_close) {
        return Ok(format!(
            "{}{line}{}",
            &manifest[..start],
            &manifest[end + 1..]
        ));
    }

    let line_start = manifest[..entry_close].rfind('\n').map_or(0, |i| i + 1);
    if manifest[line_start..entry_close].trim().is_empty() {
        // `}` on its own line: a new line before it, indented like the fields.
        let close_indent = indent_of(manifest, entry_close);
        let field_indent = format!("{close_indent}{}", unit(close_indent));
        Ok(format!(
            "{}{field_indent}{line}\n{}",
            &manifest[..line_start],
            &manifest[line_start..]
        ))
    } else {
        // A one-line entry (`a = { path = "../a" }`): keep it on one line.
        Ok(format!(
            "{} {line} {}",
            manifest[..entry_close].trim_end(),
            &manifest[entry_close..]
        ))
    }
}

/// The `{` and `}` of `name = { ... }` directly inside the block.
fn entry(manifest: &str, open: usize, close: usize, name: &str) -> Option<(usize, usize)> {
    let toks: Vec<_> = tokens(manifest)
        .into_iter()
        .filter(|&(_, at)| at > open && at < close)
        .collect();
    let mut depth = 0usize;
    for (i, &(token, _)) in toks.iter().enumerate() {
        match token {
            Token::Open => depth += 1,
            Token::Close => depth = depth.saturating_sub(1),
            Token::Word(word) if depth == 0 && word == name => {
                if let [(Token::Equals, _), (Token::Open, entry_open), ..] = toks[i + 1..] {
                    return matching(&toks[i + 2..]).map(|entry_close| (entry_open, entry_close));
                }
            }
            _ => {}
        }
    }
    None
}

/// The span of an `accept_unverified = { ... }` directly inside an entry:
/// from the key to its closing `}`.
fn existing(manifest: &str, open: usize, close: usize) -> Option<(usize, usize)> {
    let toks: Vec<_> = tokens(manifest)
        .into_iter()
        .filter(|&(_, at)| at > open && at < close)
        .collect();
    let mut depth = 0usize;
    for (i, &(token, at)) in toks.iter().enumerate() {
        match token {
            Token::Open => depth += 1,
            Token::Close => depth = depth.saturating_sub(1),
            Token::Word("accept_unverified") if depth == 0 => {
                if let [(Token::Equals, _), (Token::Open, _), ..] = toks[i + 1..] {
                    return matching(&toks[i + 2..]).map(|end| (at, end));
                }
            }
            _ => {}
        }
    }
    None
}

/// Given tokens starting at an opening bracket, the offset of its match.
fn matching(toks: &[(Token<'_>, usize)]) -> Option<usize> {
    let mut depth = 0usize;
    for &(token, at) in toks {
        match token {
            Token::Open => depth += 1,
            Token::Close if depth == 1 => return Some(at),
            Token::Close => depth -= 1,
            _ => {}
        }
    }
    None
}

/// Whether `updated` declares the same dependencies as before, with only
/// `name`'s acceptance changed to `accepted`.
fn verify(
    updated: &str,
    before: &[DependencyConfig],
    name: &str,
    accepted: &BTreeMap<String, Vec<Guarantee>>,
) -> Result<(), String> {
    let after = declared_dependencies(updated).map_err(|e| e.to_string())?;
    if after.len() != before.len() {
        return Err("other entries changed".to_string());
    }
    for old in before {
        let Some(new) = after.iter().find(|d| d.name == old.name) else {
            return Err(format!("`{}` isn't read back", old.name));
        };
        let expected = if old.name == name {
            accepted
        } else {
            &old.accept_unverified
        };
        if &new.accept_unverified != expected
            || format!("{:?}", new.source) != format!("{:?}", old.source)
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

    fn both() -> BTreeMap<String, Vec<Guarantee>> {
        BTreeMap::from([(
            "Billing".to_string(),
            vec![Guarantee::SchemaAgreement, Guarantee::EncodingAgreement],
        )])
    }

    fn check(manifest: &str, expected: &str) {
        let before = declared_dependencies(manifest).unwrap();
        let updated = with_acceptance(manifest, "billing", &both()).unwrap();
        assert_eq!(updated, expected);
        assert_eq!(
            verify(&updated, &before, "billing", &both()),
            Ok(()),
            "{updated}"
        );
    }

    #[test]
    fn a_line_goes_before_the_entrys_closing_brace() {
        check(
            &format!("{HEAD}dependencies = {{\n    billing = {{\n        path = \"../billing\"\n    }}\n    other = {{\n        path = \"../o\"\n    }}\n}}\n"),
            &format!("{HEAD}dependencies = {{\n    billing = {{\n        path = \"../billing\"\n        accept_unverified = {{ Billing = [\"schema_agreement\", \"encoding_agreement\"] }}\n    }}\n    other = {{\n        path = \"../o\"\n    }}\n}}\n"),
        );
    }

    #[test]
    fn a_one_line_entry_stays_on_one_line() {
        check(
            &format!("{HEAD}dependencies = {{\n    billing = {{ path = \"../billing\" }}\n}}\n"),
            &format!("{HEAD}dependencies = {{\n    billing = {{ path = \"../billing\" accept_unverified = {{ Billing = [\"schema_agreement\", \"encoding_agreement\"] }} }}\n}}\n"),
        );
    }

    #[test]
    fn an_existing_acceptance_is_replaced_in_place() {
        check(
            &format!("{HEAD}dependencies = {{\n    billing = {{\n        accept_unverified = {{ Billing = [\"schema_agreement\"] }} // old\n        path = \"../billing\"\n    }}\n}}\n"),
            &format!("{HEAD}dependencies = {{\n    billing = {{\n        accept_unverified = {{ Billing = [\"schema_agreement\", \"encoding_agreement\"] }} // old\n        path = \"../billing\"\n    }}\n}}\n"),
        );
    }

    #[test]
    fn a_missing_entry_is_reported() {
        let manifest = format!("{HEAD}dependencies = {{\n    other = {{ path = \"../o\" }}\n}}\n");
        assert_eq!(
            with_acceptance(&manifest, "billing", &both()),
            Err("no `billing = { ... }` entry in it".to_string())
        );
        // A key of that name nested deeper isn't the entry.
        let nested = format!("{HEAD}dependencies = {{\n    other = {{ billing = {{}} }}\n}}\n");
        assert!(with_acceptance(&nested, "billing", &both()).is_err());
    }
}
