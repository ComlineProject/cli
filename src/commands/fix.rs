//! `comline fix` — apply the fixes core attaches to diagnostics, and move a
//! package to another edition (core#102).
//!
//! The fixes are the ones the language itself asks for: a `config.idp`
//! without an edition gets `specification_version`, and every use of a word
//! the edition deprecates or removes is replaced, when it has a
//! replacement. A use with no replacement is listed for the author to
//! change by hand. With `--edition N`, the schemas are checked under edition
//! N, fixed, and `config.idp` moves to N, unless a removed word is left
//! that has no replacement.

use std::path::{Path, PathBuf};

use comline_core::diagnostics::{Diagnostic, Fix};
use comline_core::edition::{self, Edition};
use comline_core::package::config::idl::grammar::{self as config_grammar, Key};
use comline_core::schema::idl::vocabulary;
use miette::{IntoDiagnostic, Result, WrapErr};

use crate::commands::ensure_project;
use crate::ui;

const CONFIG: &str = "config.idp";

pub fn run(work_dir: &Path, to: Option<u8>, dry_run: bool) -> Result<()> {
    ensure_project(work_dir)?;
    ui::step(format!(
        "{}{}",
        if dry_run {
            "Checking what to fix"
        } else {
            "Fixing"
        },
        ui::at_path(work_dir)
    ));

    let config_path = work_dir.join(CONFIG);
    let config_source = read(&config_path)?;
    let config = config_grammar::parse(&config_source)
        .map_err(|_| miette::miette!("{CONFIG} doesn't parse; `comline check` shows where"))?;

    // The package's edition, after fixing a missing or unknown one.
    let mut config_out = config_source.clone();
    let mut fixed = 0usize;
    let current = match edition::check_config(&config_source, &config) {
        Ok(current) => current,
        Err(diagnostic) => {
            let fix = diagnostic
                .fix
                .as_ref()
                .expect("core's edition errors carry a fix");
            ui::detail(format!("{CONFIG}: {}", fix.title));
            config_out = fix.apply(&config_out);
            fixed += 1;
            Edition::LATEST
        }
    };

    let target = match to {
        None => current,
        Some(n) => {
            let target = Edition::from_number(n).ok_or_else(|| {
                miette::miette!(
                    "edition {n} isn't one this Comline knows; it knows {}",
                    Edition::known()
                )
            })?;
            if target < current {
                return Err(miette::miette!(
                    "the package is on edition {current}; moving back to {target} isn't supported"
                ));
            }
            target
        }
    };

    // Each schema under the target edition: what it deprecates or removes.
    let mut writes: Vec<(PathBuf, String)> = Vec::new();
    let mut by_hand: Vec<(PathBuf, String, Diagnostic)> = Vec::new();
    for path in schema_files(&work_dir.join("src"))? {
        let source = read(&path)?;
        let findings = edition::deprecations(&source, target, vocabulary::DEPRECATIONS);
        let (fixable, rest): (Vec<_>, Vec<_>) = findings.into_iter().partition(|d| d.fix.is_some());
        if !fixable.is_empty() {
            let fixes: Vec<&Fix> = fixable.iter().filter_map(|d| d.fix.as_ref()).collect();
            for fix in &fixes {
                ui::detail(format!(
                    "{}: {}",
                    ui::shown_path(work_dir, &path),
                    fix.title
                ));
            }
            fixed += fixes.len();
            writes.push((path.clone(), apply_all(&source, &fixes)));
        }
        by_hand.extend(rest.into_iter().map(|d| (path.clone(), source.clone(), d)));
    }

    let blocked = by_hand.iter().any(|(_, _, d)| d.code == edition::REMOVED);
    for (path, source, d) in &by_hand {
        let rendered =
            comline_core::diagnostics::render(d, &ui::shown_path(work_dir, path), source);
        if d.code == edition::REMOVED {
            ui::error(rendered)
        } else {
            ui::warn(rendered)
        }
    }

    let moving = target != current;
    if moving && !blocked {
        config_out = set_edition(&config_out, target)?;
    }

    if !dry_run {
        if config_out != config_source {
            write(&config_path, &config_out)?;
        }
        for (path, source) in &writes {
            write(path, source)?;
        }
    }

    let verb = if dry_run { "would apply" } else { "applied" };
    let fixes = |n: usize| {
        if n == 1 {
            "1 fix".to_string()
        } else {
            format!("{n} fixes")
        }
    };
    match (moving, blocked) {
        (true, true) => {
            return Err(miette::miette!(
                "edition {target} removes words used above that have no replacement; change them by \
                 hand, then run `comline fix --edition {target}` again (the package stays on edition \
                 {current}{})",
                if fixed > 0 { format!("; {verb} {} meanwhile", fixes(fixed)) } else { String::new() }
            ));
        }
        (true, false) => ui::success(format!(
            "{} edition {current} {} {target}{}",
            if dry_run {
                "Would move from"
            } else {
                "Moved from"
            },
            ui::arrow(),
            if fixed > 0 {
                format!(", {verb} {}", fixes(fixed))
            } else {
                String::new()
            }
        )),
        (false, _) if fixed == 0 && by_hand.is_empty() => ui::success("Nothing to fix"),
        (false, _) => ui::success(format!(
            "{} {}{}",
            if dry_run { "Would apply" } else { "Applied" },
            fixes(fixed),
            if by_hand.is_empty() {
                String::new()
            } else {
                format!(", {} to change by hand", by_hand.len())
            }
        )),
    }
    if blocked {
        return Err(miette::miette!(
            "the schemas use words their edition removed; change them by hand"
        ));
    }
    Ok(())
}

/// `source` with every fix applied. Fixes from one schema never overlap:
/// each replaces one token.
fn apply_all(source: &str, fixes: &[&Fix]) -> String {
    let merged = Fix {
        title: String::new(),
        edits: fixes.iter().flat_map(|f| f.edits.iter().cloned()).collect(),
    };
    merged.apply(source)
}

/// `config.idp` with `specification_version` set to `edition`. The line is
/// there: [`edition::check_config`] passed or its fix added it.
fn set_edition(source: &str, edition: Edition) -> Result<String> {
    let config = config_grammar::parse(source)
        .map_err(|_| miette::miette!("{CONFIG} doesn't parse after fixing it"))?;
    let assignment = config
        .assignments
        .iter()
        .find(|a| matches!(&a.value.key, Key::Identifier(i) if i.value == "specification_version"))
        .ok_or_else(|| miette::miette!("{CONFIG} has no `specification_version`"))?;
    let (start, end) = (assignment.span.0, assignment.span.1);
    let written = &source[start..end];
    let Some(eq) = written.find('=') else {
        return Err(miette::miette!(
            "{CONFIG}'s `specification_version` has no `=`"
        ));
    };
    let value_start =
        start + eq + 1 + (written[eq + 1..].len() - written[eq + 1..].trim_start().len());
    let value_end = start + written.trim_end().len();
    let fix = Fix::replace("", (value_start, value_end), edition.to_string());
    Ok(fix.apply(source))
}

/// Every `.ids` under `root`, sorted, so the output is stable.
fn schema_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries {
            let path = entry.into_diagnostic()?.path();
            if path.is_dir() {
                dirs.push(path);
            } else if path.extension().is_some_and(|e| e == "ids") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path)
        .into_diagnostic()
        .wrap_err_with(|| format!("reading {}", path.display()))
}

fn write(path: &Path, source: &str) -> Result<()> {
    std::fs::write(path, source)
        .into_diagnostic()
        .wrap_err_with(|| format!("writing {}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_edition_replaces_only_the_number() {
        let source = "congregation app\nspecification_version   =  1 // the edition\nid = \"x\"\n";
        let out = set_edition(source, Edition::E1).unwrap();
        assert_eq!(out, source);
    }

    #[test]
    fn apply_all_applies_every_fix_in_one_file() {
        // As two deprecations in one schema would: each replaces one token.
        let source = "struct S {\n    a: u8\n    b: u8\n}\n";
        let at = |from: usize| {
            let start = from + source[from..].find("u8").unwrap();
            (start, start + 2)
        };
        let (a, b) = (at(0), at(at(0).1));
        let first = Fix::replace("a", a, "byte");
        let second = Fix::replace("b", b, "byte");
        assert_eq!(
            apply_all(source, &[&first, &second]),
            "struct S {\n    a: byte\n    b: byte\n}\n"
        );
    }

    #[test]
    fn schema_files_finds_nested_schemas_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("src");
        std::fs::create_dir_all(src.join("a/b")).unwrap();
        for f in ["z.ids", "a/b/c.ids", "a/notes.md"] {
            std::fs::write(src.join(f), "").unwrap();
        }
        let found: Vec<_> = schema_files(&src)
            .unwrap()
            .iter()
            .map(|p| {
                p.strip_prefix(&src)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        assert_eq!(found, ["a/b/c.ids", "z.ids"]);
    }
}
