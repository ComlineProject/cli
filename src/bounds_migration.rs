//! Moving a project onto bounds on types (core#98): every string and dynamic
//! array needs a maximum, so a project written before them stops building
//! with one `bounds::unbounded` error per position.
//!
//! The build already lists every position, each with its fix. On top of
//! that the CLI offers what the design asks for: adding package defaults
//! (`bounds.string.max`, `bounds.array.max` in `config.idp`'s `settings`),
//! with maxima the author types in, never guessed. Without a terminal to ask
//! on, it prints the block to add instead.

use std::io::{BufRead, IsTerminal, Write};
use std::path::Path;

use crate::ui;

/// The types guide's section on maxima.
pub const GUIDE: &str =
    "https://comlineproject.github.io/docs/guide/idl/types/#every-string-and-list-has-a-maximum";

/// What a failed build's errors say is unbounded.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Unbounded {
    pub strings: usize,
    pub arrays: usize,
}

impl Unbounded {
    /// Read off a build's rendered errors: each `bounds::unbounded` error
    /// says "`<type>` has no maximum".
    pub fn of(rendered: &str) -> Self {
        let mut found = Self::default();
        for line in rendered
            .lines()
            .filter(|l| l.contains("[bounds::unbounded]"))
        {
            let Some(end) = line.find("` has no maximum") else {
                continue;
            };
            let Some(start) = line[..end].rfind('`') else {
                continue;
            };
            let ty = &line[start + 1..end];
            if ty == "string" || ty.starts_with("char") {
                found.strings += 1;
            } else {
                found.arrays += 1;
            }
        }
        found
    }

    pub fn is_empty(&self) -> bool {
        self.strings == 0 && self.arrays == 0
    }

    pub fn total(&self) -> usize {
        self.strings + self.arrays
    }
}

/// The `settings` block giving the package these defaults.
fn settings_block(string: Option<&str>, array: Option<&str>) -> String {
    let mut block = String::from("settings = {\n    bounds = {\n");
    if let Some(n) = string {
        block.push_str(&format!(
            "        string = {{\n            max = {n}\n        }}\n"
        ));
    }
    if let Some(n) = array {
        block.push_str(&format!(
            "        array = {{\n            max = {n}\n        }}\n"
        ));
    }
    block.push_str("    }\n}\n");
    block
}

/// Whether `config.idp` already has a top-level `settings` key, which a
/// second one appended at the end would clash with.
fn has_settings(config: &str) -> bool {
    config.lines().any(|l| {
        l.strip_prefix("settings")
            .is_some_and(|rest| rest.is_empty() || rest.starts_with([' ', '=', '{']))
    })
}

/// Whether there's someone to ask: a terminal both ways, and not `--plain`.
pub fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal() && !ui::plain()
}

/// What to do about them, for the error's help.
pub fn hint(unbounded: &Unbounded) -> String {
    let n = unbounded.total();
    let values = if n == 1 { "value has" } else { "values have" };
    let block = settings_block(
        (unbounded.strings > 0).then_some("N"),
        (unbounded.arrays > 0).then_some("N"),
    );
    format!(
        "{n} {values} no maximum: every string and list needs one. Bound each in its type \
         (`char[max N]`, `T[max N]`), or give the package defaults for those that don't say, \
         in config.idp (with your own maxima for N):\n\n{block}\nSee {GUIDE}"
    )
}

/// Offer to write package defaults from the author's input. Returns `true`
/// when it wrote `config.idp`, so the caller can build again.
pub fn offer(work_dir: &Path, unbounded: &Unbounded) -> bool {
    offer_defaults(work_dir, unbounded).unwrap_or_else(|e| {
        ui::warn(format!("couldn't add the defaults: {e}"));
        false
    })
}

/// Ask for each default the errors call for; append them to `config.idp`.
fn offer_defaults(work_dir: &Path, unbounded: &Unbounded) -> std::io::Result<bool> {
    let path = work_dir.join("config.idp");
    let config = std::fs::read_to_string(&path)?;
    if has_settings(&config) {
        ui::note(
            "config.idp already has `settings`: add `bounds.string.max` / `bounds.array.max` to it",
        );
        return Ok(false);
    }
    if !ask_yes("Add package defaults to config.idp now?")? {
        return Ok(false);
    }
    let string = if unbounded.strings > 0 {
        ask_max("Maximum characters in a `string` that doesn't say (empty to skip)")?
    } else {
        None
    };
    let array = if unbounded.arrays > 0 {
        ask_max("Maximum items in a list that doesn't say (empty to skip)")?
    } else {
        None
    };
    if string.is_none() && array.is_none() {
        return Ok(false);
    }
    let mut text = config;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push_str(
        "\n// Every string and list has a maximum (bounds on types); these are the\n\
         // defaults where a type writes none.\n",
    );
    text.push_str(&settings_block(string.as_deref(), array.as_deref()));
    std::fs::write(&path, text)?;
    ui::success("Added the defaults to config.idp");
    Ok(true)
}

fn prompt(question: &str) -> std::io::Result<String> {
    let mut stderr = std::io::stderr();
    write!(stderr, "{question} ")?;
    stderr.flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    Ok(line.trim().to_string())
}

fn ask_yes(question: &str) -> std::io::Result<bool> {
    let answer = prompt(&format!("{question} [y/N]"))?;
    Ok(matches!(answer.as_str(), "y" | "Y" | "yes"))
}

/// A maximum from 1 to `u32::MAX`, asked until it is one or left empty.
fn ask_max(question: &str) -> std::io::Result<Option<String>> {
    loop {
        let answer = prompt(&format!("{question}:"))?;
        if answer.is_empty() {
            return Ok(None);
        }
        match answer.parse::<u32>() {
            Ok(n) if n > 0 => return Ok(Some(n.to_string())),
            _ => ui::warn("a maximum is a whole number from 1 to 4294967295"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_strings_and_arrays_from_the_errors() {
        let rendered = "\
[bounds::unbounded] Error: struct `A`, field `b`: `string` has no maximum
  Help: write `char[max N]`, or set `bounds.string.max` in settings
[bounds::unbounded] Error: struct `A`, field `c`: `u32[]` has no maximum
[bounds::unbounded-depth] Error: struct `Tree` can contain itself, and has no depth limit
";
        assert_eq!(
            Unbounded::of(rendered),
            Unbounded {
                strings: 1,
                arrays: 1
            }
        );
    }

    #[test]
    fn a_settings_key_is_found_only_at_the_top_level() {
        assert!(has_settings("congregation a\nsettings = {\n}\n"));
        assert!(!has_settings("congregation a\n// settings = {\n"));
        assert!(!has_settings(
            "congregation a\nx = {\n    settings_like = 1\n}\n"
        ));
    }
}
