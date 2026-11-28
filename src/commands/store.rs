//! `comline store` — what happens to `.comline/`, the version history, when
//! the IR format it is stored in changes (core#268, `ir-migrations` in the
//! docs repo).
//!
//! The rule behind every command here: **nothing is deleted or changed
//! without the user doing so.** Core is a library that never decides to
//! migrate or reset; each of its functions that changes a store takes a
//! [`Consent`], and this module passes one only because the user ran the
//! command (`Consent::Command`), or answered yes to the exact question it
//! showed (`Consent::Confirmed`). There is no flag or variable that makes a
//! command migrate or reset on its own.
//!
//! - `status`, `verify` and `migrate --check` only read.
//! - `migrate` is the migration, and finishes an interrupted one.
//! - `prune` removes the backups a migration or a reset left, once the store
//!   that replaced them passes `verify`: on a terminal it lists them and asks;
//!   without one it needs `--force`.
//! - `reset` moves a store aside (nothing is deleted): on a terminal it shows
//!   what it will do and asks; without one it needs `--force`.
//!
//! `build` and `release` stop on a store they can't use as it is. On a
//! terminal they ask whether to run `comline store migrate` ([`offer_migration`])
//! and carry on if the user says yes; otherwise they fail with core's message,
//! which names the command. A reset is never offered.

use std::io::{self, BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use comline_core::package::build::cas::migrate::{
    backups, resetting_dir, Check, Consent, Engine, MigrateOptions, Migrated, Report,
};
use comline_core::package::build::cas::object_store::{StoreError, StoreState};
use miette::{IntoDiagnostic, Result, WrapErr};

use crate::commands::ensure_project;
use crate::error::{self, CliError};
use crate::ui;

/// The only command a store error ever offers to run.
const MIGRATE_COMMAND: &str = "comline store migrate";

/// What `comline store migrate` was given.
pub struct MigrateFlags {
    pub check: bool,
    pub assume_format: Option<u32>,
    pub map: Option<PathBuf>,
    pub restart: bool,
}

/// `comline store status`: read-only.
pub fn status(work_dir: &Path) -> Result<()> {
    ensure_project(work_dir)?;
    let status = Engine::current()
        .status(work_dir)
        .map_err(|e| error::from_core("couldn't read the store", e))?;

    ui::step(format!("Store{}", ui::at_path(work_dir)));
    match status.stamp {
        Some(n) => ui::detail(format!(
            "IR format {n} (this comline writes format {})",
            status.target
        )),
        None => ui::detail(format!(
            "no format stamp (this comline writes format {})",
            status.target
        )),
    }
    match &status.classification {
        Ok(StoreState::Fresh) => ui::note("no version history yet: the next build starts it"),
        Ok(StoreState::Current) => ui::success("the store is in the current format"),
        Err(refusal) => ui::warn(refusal.to_string()),
    }

    match &status.versions {
        Ok(versions) if versions.is_empty() => {}
        Ok(versions) => {
            ui::group(format!("{} version(s), oldest first:", versions.len()));
            for version in versions {
                let kind = if version.release { "release" } else { "build" };
                let unknowns = match version.unknowns {
                    Some(0) => String::new(),
                    Some(n) => format!(", {n} unknown"),
                    None => ", unknowns not counted (its format can't be decoded here)".into(),
                };
                ui::detail(format!(
                    "{}  {}  {kind}{unknowns}",
                    version.version,
                    &version.hash.to_hex()[..12]
                ));
            }
        }
        Err(reason) => ui::warn(format!("the history can't be listed: {reason}")),
    }

    if !status.backups.is_empty() {
        ui::group("backups, kept until `comline store prune`:");
        for backup in &status.backups {
            ui::detail(ui::shown_path(work_dir, &backup.dir));
        }
    }
    Ok(())
}

/// `comline store migrate`.
pub fn migrate(work_dir: &Path, flags: MigrateFlags) -> Result<()> {
    ensure_project(work_dir)?;
    let options = MigrateOptions {
        assume_format: flags.assume_format,
        restart: flags.restart,
    };
    if flags.check {
        return check(work_dir, &options);
    }

    // The user ran the command: that is the consent.
    let migrated = migrate_now(work_dir, &options, Consent::Command)?;
    if let Some(path) = &flags.map {
        match migrated {
            Some(report) => {
                std::fs::write(path, report.map_file())
                    .into_diagnostic()
                    .wrap_err_with(|| {
                        format!(
                            "the store is migrated, but couldn't write the hash map to `{}`",
                            path.display()
                        )
                    })?;
                ui::detail(format!(
                    "wrote the old and new hash of {} commit(s) to {}",
                    report.map.len(),
                    path.display()
                ));
            }
            None => ui::note("no hash map written: nothing was migrated"),
        }
    }
    Ok(())
}

/// `comline store migrate --check`: what would happen, and no write. The
/// exit code says which of the three it is.
fn check(work_dir: &Path, options: &MigrateOptions) -> Result<()> {
    let check = Engine::current().check(work_dir, options);
    match &check {
        Check::NothingToDo => ui::success("Nothing to migrate: the store is in the current format"),
        // The plan is this command's output: stdout, whatever `--quiet` says.
        Check::WouldMigrate(plan) => println!("{plan}"),
        Check::Refused(refusal) => ui::error(format!("would be refused: {refusal:#}")),
    }
    match check.exit_code() {
        0 => Ok(()),
        code => Err(CliError::Exit(code).into()),
    }
}

/// Run the migration and say what it did. `None`: the store was already in
/// the current format (or there is none).
fn migrate_now(
    work_dir: &Path,
    options: &MigrateOptions,
    consent: Consent,
) -> Result<Option<Report>> {
    let migrated = Engine::current()
        .migrate(work_dir, options, consent)
        .map_err(|e| error::from_core("couldn't migrate the store", e))?;
    match migrated {
        Migrated::NothingToDo => {
            ui::success("Nothing to migrate: the store is in the current format");
            Ok(None)
        }
        Migrated::Done(report) => {
            ui::success(format!(
                "Migrated the store from IR format {} to {}{}",
                report.from,
                report.to,
                if report.resumed {
                    " (finished an interrupted migration)"
                } else {
                    ""
                }
            ));
            ui::detail(format!("{} version(s)", report.versions));
            ui::detail(format!(
                "the old store is kept as {}: `comline store prune` removes it when you no \
                 longer need it",
                ui::shown_path(work_dir, &report.backup)
            ));
            Ok(Some(report))
        }
    }
}

/// `comline store verify`: read-only.
pub fn verify(work_dir: &Path) -> Result<()> {
    ensure_project(work_dir)?;
    let report = Engine::current()
        .verify(work_dir)
        .map_err(|e| error::from_core("the store failed verification", e))?;
    if report.versions == 0 {
        ui::success("Nothing to verify: there is no version history yet");
        return Ok(());
    }
    ui::success(format!(
        "The store is sound: IR format {}, {} version(s), {} object(s)",
        report.format, report.versions, report.objects
    ));
    for (version, hash) in &report.not_canonical {
        ui::note(format!(
            "{version} ({}) decodes, but this comline would encode it differently: it was \
             written as it is and is left alone",
            &hash.to_hex()[..12]
        ));
    }
    Ok(())
}

/// `comline store prune`: remove the backups. On a terminal the user is
/// shown exactly which ones and asked; without one `--force` is the way to
/// say it. Core removes them only if the store that replaced them passes
/// `verify`, and says why when it won't.
pub fn prune(work_dir: &Path, force: bool) -> Result<()> {
    ensure_project(work_dir)?;
    let found = backups(work_dir).map_err(|e| error::from_core("couldn't list the backups", e))?;
    if found.is_empty() {
        ui::success("No backups to remove");
        return Ok(());
    }

    let consent = if force {
        Consent::Command
    } else if interactive() {
        let mut output = io::stdout();
        writeln!(output, "These backups would be removed:").into_diagnostic()?;
        for backup in &found {
            let format = backup.format.map_or_else(
                || "unknown IR format".to_string(),
                |n| format!("IR format {n}"),
            );
            writeln!(
                output,
                "  {} ({format})",
                ui::shown_path(work_dir, &backup.dir)
            )
            .into_diagnostic()?;
        }
        let question = format!("Remove {} backup(s)? This cannot be undone.", found.len());
        let stdin = io::stdin();
        if !ask(&mut stdin.lock(), &mut output, &question)? {
            ui::note("Cancelled: nothing was removed");
            return Ok(());
        }
        Consent::Confirmed
    } else {
        return Err(CliError::PruneConfirmationRequired.into());
    };

    let pruned = Engine::current()
        .prune(work_dir, consent)
        .map_err(|e| error::from_core("couldn't prune the backups", e))?;
    ui::success(format!("Removed {} backup(s)", pruned.removed.len()));
    for backup in &pruned.removed {
        ui::detail(ui::shown_path(work_dir, &backup.dir));
    }
    Ok(())
}

/// `comline store reset`: move the store aside and start a new history.
/// Deletes nothing. On a terminal the user is shown the exact action and
/// asked; without one `--force` is the way to say it.
pub fn reset(work_dir: &Path, force: bool) -> Result<()> {
    ensure_project(work_dir)?;
    let mut engine = Engine::current();

    let consent = match engine.reset_plan(work_dir) {
        Ok(plan) if force => {
            ui::note(plan.describe());
            Consent::Command
        }
        Ok(plan) if interactive() => {
            let stdin = io::stdin();
            let question = format!("{}\nReset the store?", plan.describe());
            if !ask(&mut stdin.lock(), &mut io::stdout(), &question)? {
                ui::note("Cancelled: nothing was changed");
                return Ok(());
            }
            Consent::Confirmed
        }
        Ok(_) => return Err(CliError::ResetConfirmationRequired.into()),
        // An earlier reset moved the store aside and stopped before the last
        // rename: all that is left is that rename, which is what the error
        // that sent the user here asked them to run, with nothing to confirm.
        Err(_) if finishing_a_reset(work_dir) => Consent::Command,
        Err(refusal) => return Err(error::from_core("can't reset the store", refusal)),
    };

    let report = engine
        .reset(work_dir, consent)
        .map_err(|e| error::from_core("couldn't reset the store", e))?;
    match &report.backup {
        Some(backup) => {
            ui::success(format!(
                "Moved the store aside to {}: the next build starts a new history",
                ui::shown_path(work_dir, backup)
            ));
            ui::detail("nothing was deleted; `comline store prune` removes the backup");
        }
        None => ui::success("Finished an interrupted reset"),
    }
    Ok(())
}

/// A reset whose first moves were made: `.comline.resetting/` is there and
/// `.comline/` is not.
fn finishing_a_reset(work_dir: &Path) -> bool {
    resetting_dir(work_dir).exists() && !work_dir.join(".comline").exists()
}

/// Whether there is a person to ask: stdin and stdout are both a terminal.
fn interactive() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

/// Put `question` and `[y/N]` to the user; only `y` or `yes` is a yes.
fn ask(input: &mut impl BufRead, output: &mut impl Write, question: &str) -> Result<bool> {
    write!(output, "{question} [y/N] ").into_diagnostic()?;
    output.flush().into_diagnostic()?;
    let mut line = String::new();
    input.read_line(&mut line).into_diagnostic()?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// A command stopped on a store (`report` is its error): on a terminal, ask
/// whether to run `comline store migrate`, and run it if the user says yes.
/// `Ok(true)`: it ran, so the command can go on. `Ok(false)`: nothing was
/// asked, or the answer was no, and nothing was done: the command fails with
/// `report`, a precondition (exit 2), as it does with no terminal (CI).
///
/// Only what core offers is asked ([`StoreError::offer`]), which is a
/// migration and never a reset.
pub fn offer_migration(work_dir: &Path, report: &miette::Report) -> Result<bool> {
    let Some(CliError::Store(refusal)) = report.downcast_ref::<CliError>() else {
        return Ok(false);
    };
    let stdin = io::stdin();
    offer_with(
        refusal,
        interactive(),
        &mut stdin.lock(),
        &mut io::stdout(),
        || migrate_now(work_dir, &MigrateOptions::default(), Consent::Confirmed).map(|_| ()),
    )
}

/// [`offer_migration`] with the terminal, the user's answers and the
/// migration passed in. `migrate` runs only on a yes to the question.
fn offer_with(
    refusal: &StoreError,
    interactive: bool,
    input: &mut impl BufRead,
    output: &mut impl Write,
    migrate: impl FnOnce() -> Result<()>,
) -> Result<bool> {
    let Some(offer) = refusal.offer().filter(|o| o.command == MIGRATE_COMMAND) else {
        return Ok(false);
    };
    if !interactive || !ask(input, output, &offer.question)? {
        return Ok(false);
    }
    migrate()?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::io::Cursor;

    use comline_core::package::build::cas::object_store::Interrupted;

    use super::*;

    /// Run [`offer_with`] answering `answer`; the output, whether it
    /// returned yes, and how many times the migration ran.
    fn offered(refusal: &StoreError, interactive: bool, answer: &str) -> (String, bool, usize) {
        let ran = Cell::new(0);
        let mut output = Vec::new();
        let went_on = offer_with(
            refusal,
            interactive,
            &mut Cursor::new(answer.as_bytes().to_vec()),
            &mut output,
            || {
                ran.set(ran.get() + 1);
                Ok(())
            },
        )
        .unwrap();
        (String::from_utf8(output).unwrap(), went_on, ran.get())
    }

    const NEEDS: StoreError = StoreError::NeedsMigration { found: 2 };
    const INTERRUPTED: StoreError = StoreError::Interrupted {
        what: Interrupted::Migration,
    };

    #[test]
    fn a_yes_runs_the_migration_the_question_names() {
        for refusal in [NEEDS, INTERRUPTED] {
            for yes in ["y\n", "yes\n", "  YES \n"] {
                let (output, went_on, ran) = offered(&refusal, true, yes);
                assert!(went_on && ran == 1, "{refusal:?} {yes:?}");
                assert!(output.contains("`comline store migrate`"), "{output}");
                assert!(output.ends_with("[y/N] "), "{output}");
            }
        }
    }

    #[test]
    fn anything_but_a_yes_does_nothing() {
        for answer in ["n\n", "\n", "", "no\n", "yep\n", "sure\n"] {
            let (_, went_on, ran) = offered(&NEEDS, true, answer);
            assert!(!went_on && ran == 0, "{answer:?}");
        }
    }

    #[test]
    fn with_no_terminal_nothing_is_asked_or_run() {
        for refusal in [NEEDS, INTERRUPTED] {
            // Even an answer waiting on stdin is never read.
            let (output, went_on, ran) = offered(&refusal, false, "y\n");
            assert!(!went_on && ran == 0, "{refusal:?}");
            assert_eq!(output, "");
        }
    }

    /// A reset is the user's own command: no refusal offers one, on a
    /// terminal or not, whatever the answer.
    #[test]
    fn a_reset_is_never_offered() {
        for refusal in [
            StoreError::Older { found: 1 },
            StoreError::Unstamped,
            StoreError::UnreadableStamp { reason: "x".into() },
            StoreError::InvalidStamp { text: "x".into() },
            StoreError::Interrupted {
                what: Interrupted::Reset,
            },
            StoreError::Newer { found: 99 },
            StoreError::MergedFormats { ours: 3, theirs: 4 },
        ] {
            let (output, went_on, ran) = offered(&refusal, true, "y\n");
            assert!(!went_on && ran == 0, "{refusal:?}");
            assert_eq!(output, "", "{refusal:?}");
        }
    }

    #[test]
    fn a_failing_migration_is_the_commands_error() {
        let result = offer_with(
            &NEEDS,
            true,
            &mut Cursor::new(b"y\n".to_vec()),
            &mut Vec::new(),
            || Err(miette::miette!("disk full")),
        );
        assert!(result.is_err());
    }
}
