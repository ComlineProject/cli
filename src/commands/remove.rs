//! `comline deps remove` — take a dependency out of `config.idp`.
//!
//! Only the dependency's entry goes: every other line of the file is left as
//! written. A dependency a schema still imports (`use NAME::…`) isn't removed,
//! since that would leave the build broken; the error lists each use.

use std::path::{Path, PathBuf};

use comline_core::package::config::dependency::DependencyConfig;
use comline_core::package::layout::SCHEMAS_DIR;
use comline_core::schema::idl::grammar::{Declaration, UsePath};
use comline_core::schema::idl::parse::parse_schema;
use miette::{IntoDiagnostic, Result, WrapErr};

use crate::commands::accept::entry_at;
use crate::commands::add::{declared_dependencies, declared_named, dependencies_block, indent_of};
use crate::commands::ensure_project;
use crate::ui;

/// A schema's `use` of a dependency.
#[derive(Debug, PartialEq)]
pub struct Use {
    /// The schema, relative to the package's directory.
    pub file: PathBuf,
    /// 1-based.
    pub line: usize,
    /// The statement as written, on its first line.
    pub text: String,
}

pub fn run(work_dir: &Path, name: &str, dry_run: bool) -> Result<()> {
    ensure_project(work_dir)?;

    let manifest_path = work_dir.join("config.idp");
    let manifest = std::fs::read_to_string(&manifest_path)
        .into_diagnostic()
        .wrap_err("failed to read config.idp")?;
    let declared = declared_dependencies(&manifest)?;
    declared_named(&declared, name)?;

    let uses = uses_of(work_dir, name)?;
    if !uses.is_empty() {
        let lines: Vec<String> = uses
            .iter()
            .map(|u| format!("  {}:{}  {}", slashed(&u.file), u.line, u.text))
            .collect();
        return Err(miette::miette!(
            "can't remove `{name}`: {} still import{} it:\n{}\n\nremove or change those first; \
             config.idp is unchanged",
            match uses.len() {
                1 => "1 schema statement".to_string(),
                n => format!("{n} schema statements"),
            },
            if uses.len() == 1 { "s" } else { "" },
            lines.join("\n")
        ));
    }

    let (updated, removed) = without_dependency(&manifest, name)
        .and_then(|(updated, removed)| {
            verify(&updated, &declared, name)?;
            Ok((updated, removed))
        })
        .map_err(|problem| {
            miette::miette!(
                "couldn't remove the entry from config.idp safely ({problem}); remove `{name}`'s \
                 entry from the `dependencies` block by hand"
            )
        })?;

    if dry_run {
        ui::step(format!("Would remove {name} from config.idp"));
        for line in removed.trim_end().lines() {
            ui::detail(format!("- {}", line.trim_end()));
        }
        ui::success("Dry run complete — config.idp is unchanged");
        return Ok(());
    }

    std::fs::write(&manifest_path, updated)
        .into_diagnostic()
        .wrap_err("failed to write config.idp")?;
    ui::success(format!("Removed {name} from config.idp"));
    Ok(())
}

/// Every statement in the package's own schemas that imports from `name`
/// (`use name::…`, with a glob, a list or an alias; the older `import name::…`
/// too), in file order. A schema that doesn't parse can't be told, so it's an
/// error rather than a guess.
pub fn uses_of(work_dir: &Path, name: &str) -> Result<Vec<Use>> {
    let mut files = Vec::new();
    schema_files(&work_dir.join(SCHEMAS_DIR), &mut files)?;
    files.sort();

    let mut uses = Vec::new();
    for file in files {
        let source = std::fs::read_to_string(&file)
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to read {}", file.display()))?;
        let shown = file.strip_prefix(work_dir).unwrap_or(&file).to_path_buf();
        let document = parse_schema(&source).map_err(|_| {
            miette::miette!(
                "can't tell whether {} imports `{name}`: it doesn't parse (`comline check` shows \
                 why); fix it first",
                slashed(&shown)
            )
        })?;
        for declaration in &document.0 {
            let path = match &declaration.value {
                Declaration::Use(u) => match &u.path {
                    UsePath::Absolute(path) => path.as_str(),
                    UsePath::Glob(glob) => glob.path.as_str(),
                    UsePath::Multi(multi) => multi.path.as_str(),
                    // `self`, `parent` and `package` name this package.
                    UsePath::Relative(_) | UsePath::RelativeGlob(_) | UsePath::RelativeMulti(_) => {
                        continue
                    }
                },
                Declaration::Import(import) => import.path.as_str(),
                _ => continue,
            };
            if path.split("::").next() != Some(name) {
                continue;
            }
            let at = declaration.span.0;
            uses.push(Use {
                file: shown.clone(),
                line: source[..at].matches('\n').count() + 1,
                text: source[at..].lines().next().unwrap_or("").trim().to_string(),
            });
        }
    }
    Ok(uses)
}

/// Every `.ids` file under `dir`, the way the build looks for schemas.
fn schema_files(dir: &Path, found: &mut Vec<PathBuf>) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Ok(());
    };
    for entry in entries {
        let path = entry.into_diagnostic()?.path();
        if path.is_dir() {
            schema_files(&path, found)?;
        } else if path.extension().is_some_and(|e| e == "ids") {
            found.push(path);
        }
    }
    Ok(())
}

fn slashed(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// `manifest` without `name`'s entry, and the entry's text as it was. The
/// entry's own lines go (and a trailing `//` comment on its last one), with
/// the blank line that separated it from its neighbour; an entry sharing its
/// lines with others is cut out alone. Every other byte is left as written.
pub fn without_dependency(manifest: &str, name: &str) -> Result<(String, String), String> {
    let (block_open, block_close) =
        dependencies_block(manifest).ok_or("no `dependencies` block")?;
    let (name_at, _, close) = entry_at(manifest, block_open, block_close, name)
        .ok_or_else(|| format!("no `{name} = {{ ... }}` entry in it"))?;

    let after = close + 1;
    let line_start = manifest[..name_at].rfind('\n').map_or(0, |i| i + 1);
    let eol = manifest[after..]
        .find('\n')
        .map_or(manifest.len(), |i| after + i);
    let line_end = (eol + 1).min(manifest.len());
    let own_lines = manifest[line_start..name_at].trim().is_empty()
        && (manifest[after..eol].trim().is_empty()
            || manifest[after..eol].trim_start().starts_with("//"));

    let (mut start, mut end) = (line_start, line_end);
    if own_lines {
        let before = &manifest[..line_start];
        let previous = before
            .strip_suffix('\n')
            .map(|p| &p[p.rfind('\n').map_or(0, |i| i + 1)..]);
        let next = manifest[line_end..]
            .lines()
            .next()
            .filter(|_| line_end < manifest.len());
        let previous_blank = previous.is_some_and(|l| l.trim().is_empty());
        let next_blank = next.is_some_and(|l| l.trim().is_empty());
        let next_closes = next.is_some_and(|l| l.trim_start().starts_with('}'));
        if previous_blank && (next_blank || next_closes) {
            start = before.len() - previous.map_or(0, |l| l.len() + 1);
        } else if previous.is_some_and(|l| l.trim_end().ends_with('{')) && next_blank {
            end = line_end + next.map_or(0, |l| l.len() + 1);
        }
    } else {
        start = name_at;
        end = after;
        let is_space = |c: char| c == ' ' || c == '\t';
        let trailing = manifest[end..].len() - manifest[end..].trim_start_matches(is_space).len();
        if trailing > 0 {
            end += trailing;
        } else {
            start -= manifest[..start].len() - manifest[..start].trim_end_matches(is_space).len();
        }
    }

    // The entry's text, with its own indentation taken off.
    let indent = indent_of(manifest, name_at);
    let removed = manifest[name_at..after]
        .lines()
        .enumerate()
        .map(|(i, line)| match i {
            0 => line,
            _ => line.strip_prefix(indent).unwrap_or(line),
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok((
        format!("{}{}", &manifest[..start], &manifest[end..]),
        removed,
    ))
}

/// Whether `updated` declares what was declared before, minus `name`.
fn verify(updated: &str, before: &[DependencyConfig], name: &str) -> Result<(), String> {
    let after = declared_dependencies(updated).map_err(|e| e.to_string())?;
    if after.len() + 1 != before.len() || after.iter().any(|d| d.name == name) {
        return Err("it still declares it, or lost another entry".to_string());
    }
    for old in before.iter().filter(|d| d.name != name) {
        let Some(new) = after.iter().find(|d| d.name == old.name) else {
            return Err(format!("`{}` isn't read back", old.name));
        };
        if new.accept_unverified != old.accept_unverified
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

    fn entry(name: &str) -> String {
        format!("    {name} = {{\n        path = \"../{name}\"\n    }}\n")
    }

    fn without(manifest: &str, name: &str) -> String {
        let before = declared_dependencies(manifest).unwrap();
        let (updated, removed) = without_dependency(manifest, name).unwrap();
        assert!(removed.starts_with(&format!("{name} = {{")), "{removed}");
        assert_eq!(verify(&updated, &before, name), Ok(()), "{updated}");
        updated
    }

    #[test]
    fn an_entrys_lines_go_and_the_rest_stays() {
        let manifest = format!(
            "{HEAD}\ndependencies = {{\n{}{}{}}}\n\n// after\n",
            entry("a"),
            entry("b"),
            entry("c")
        );
        assert_eq!(
            without(&manifest, "b"),
            format!(
                "{HEAD}\ndependencies = {{\n{}{}}}\n\n// after\n",
                entry("a"),
                entry("c")
            )
        );
        assert_eq!(
            without(&manifest, "a"),
            format!(
                "{HEAD}\ndependencies = {{\n{}{}}}\n\n// after\n",
                entry("b"),
                entry("c")
            )
        );
        assert_eq!(
            without(&manifest, "c"),
            format!(
                "{HEAD}\ndependencies = {{\n{}{}}}\n\n// after\n",
                entry("a"),
                entry("b")
            )
        );
    }

    #[test]
    fn the_blank_line_that_separated_it_goes_too() {
        let manifest = format!(
            "{HEAD}dependencies = {{\n{}\n{}\n{}}}\n",
            entry("a"),
            entry("b"),
            entry("c")
        );
        assert_eq!(
            without(&manifest, "b"),
            format!(
                "{HEAD}dependencies = {{\n{}\n{}}}\n",
                entry("a"),
                entry("c")
            )
        );
        assert_eq!(
            without(&manifest, "a"),
            format!(
                "{HEAD}dependencies = {{\n{}\n{}}}\n",
                entry("b"),
                entry("c")
            )
        );
        assert_eq!(
            without(&manifest, "c"),
            format!(
                "{HEAD}dependencies = {{\n{}\n{}}}\n",
                entry("a"),
                entry("b")
            )
        );
    }

    #[test]
    fn the_last_entry_leaves_an_empty_block() {
        let manifest = format!("{HEAD}dependencies = {{\n{}}}\n", entry("a"));
        assert_eq!(
            without(&manifest, "a"),
            format!("{HEAD}dependencies = {{\n}}\n")
        );
    }

    #[test]
    fn a_trailing_comment_goes_with_its_entry_and_tabs_work() {
        let manifest = format!(
            "{HEAD}dependencies = {{\n\ta = {{\n\t\tpath = \"../a\"\n\t}} // old\n\tb = {{ path = \"../b\" }}\n}}\n"
        );
        assert_eq!(
            without(&manifest, "a"),
            format!("{HEAD}dependencies = {{\n\tb = {{ path = \"../b\" }}\n}}\n")
        );
    }

    #[test]
    fn entries_sharing_a_line_are_cut_out_alone() {
        let manifest = format!(
            "{HEAD}dependencies = {{ a = {{ path = \"../a\" }} b = {{ path = \"../b\" }} }}\n"
        );
        assert_eq!(
            without(&manifest, "a"),
            format!("{HEAD}dependencies = {{ b = {{ path = \"../b\" }} }}\n")
        );
        assert_eq!(
            without(&manifest, "b"),
            format!("{HEAD}dependencies = {{ a = {{ path = \"../a\" }} }}\n")
        );
    }

    #[test]
    fn a_missing_entry_is_reported() {
        let manifest = format!("{HEAD}dependencies = {{\n{}}}\n", entry("a"));
        assert_eq!(
            without_dependency(&manifest, "b"),
            Err("no `b = { ... }` entry in it".to_string())
        );
        assert_eq!(
            without_dependency(HEAD, "a"),
            Err("no `dependencies` block".to_string())
        );
    }

    #[test]
    fn uses_are_found_by_the_parser_not_by_text() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(temp.path().join("src/nested")).unwrap();
        std::fs::write(
            temp.path().join("src/a.ids"),
            "// use shared::models::Old\nuse shared::models::Thing\nuse shared_types::X\n",
        )
        .unwrap();
        std::fs::write(
            temp.path().join("src/nested/b.ids"),
            "use shared::models::*\nuse shared::models::{A, B} \nuse package::shared::C\n",
        )
        .unwrap();
        let found: Vec<(String, usize)> = uses_of(temp.path(), "shared")
            .unwrap()
            .into_iter()
            .map(|u| (slashed(&u.file), u.line))
            .collect();
        assert_eq!(
            found,
            vec![
                ("src/a.ids".to_string(), 2),
                ("src/nested/b.ids".to_string(), 1),
                ("src/nested/b.ids".to_string(), 2),
            ]
        );
    }
}
