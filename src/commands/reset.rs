//! `comline reset` — discard the version history.
//!
//! Deletes `.comline/` (every frozen version and the commit chain) plus
//! generated code. Irreversible, and — since there is no `publish` yet —
//! nothing else holds the history, so the next `build` starts over at 0.0.1.
//! Guarded by a typed confirmation unless `--force`.

use std::io::{self, IsTerminal, Write};
use std::path::{Path, PathBuf};

use miette::{IntoDiagnostic, Result, WrapErr};

use crate::commands::{clean, ensure_project};
use crate::error::CliError;
use crate::history;
use crate::ui;

pub fn run(work_dir: &Path, force: bool, dry_run: bool) -> Result<()> {
    ensure_project(work_dir)?;

    ui::step(format!("Resetting{}", ui::at_path(work_dir)));

    let cas = work_dir.join(".comline");
    let mut targets: Vec<PathBuf> = Vec::new();
    if cas.exists() {
        targets.push(cas);
    }
    targets.extend(clean::generated_files(work_dir));
    targets.sort();
    targets.dedup();

    if targets.is_empty() {
        ui::success("Nothing to reset");
        return Ok(());
    }

    // History summary for the prompt — best effort; the store may exist but be
    // unreadable, and the reset should still be allowed to clear it.
    let summary = match history::load(work_dir).ok().as_deref() {
        Some([head, rest @ ..]) => {
            format!(
                "{} version(s), currently {}",
                rest.len() + 1,
                head.commit.version
            )
        }
        _ => "an unreadable or empty store".to_string(),
    };

    if dry_run {
        for target in &targets {
            let shown = target.strip_prefix(work_dir).unwrap_or(target).display();
            ui::detail(format!("would remove {shown}"));
        }
        ui::note(format!("would discard {summary}"));
        ui::success("Dry run complete — nothing was deleted");
        return Ok(());
    }

    if !force && !confirm(&summary)? {
        return Ok(());
    }

    for target in &targets {
        let shown = target.strip_prefix(work_dir).unwrap_or(target).display();
        remove(target).wrap_err_with(|| format!("failed to remove `{}`", target.display()))?;
        ui::detail(format!("removed {shown}"));
    }

    ui::success("Reset complete — the next `build` starts a new history at 0.0.1");
    Ok(())
}

/// Typed confirmation. Returns `Ok(false)` if the user declines. Refuses with a
/// precondition error when stdin is not a terminal — a script that means it
/// passes `--force`.
fn confirm(summary: &str) -> Result<bool> {
    if !io::stdin().is_terminal() {
        return Err(CliError::ConfirmationRequired.into());
    }

    ui::warn(format!(
        "This permanently deletes the version history ({summary}) and cannot be undone."
    ));
    print!("Type `reset` to confirm: ");
    io::stdout().flush().into_diagnostic()?;

    let mut line = String::new();
    io::stdin().read_line(&mut line).into_diagnostic()?;

    if line.trim() == "reset" {
        Ok(true)
    } else {
        ui::note("Cancelled — nothing was deleted");
        Ok(false)
    }
}

fn remove(path: &Path) -> Result<()> {
    if path.is_dir() {
        std::fs::remove_dir_all(path).into_diagnostic()
    } else {
        std::fs::remove_file(path).into_diagnostic()
    }
}
