//! Error type and process exit codes.
//!
//! Exit codes are part of the CLI's contract with CI and shell scripts, so they
//! are fixed here rather than left to `main`'s `Result` sugar:
//!
//! | code | meaning                                             |
//! |------|-----------------------------------------------------|
//! | 0    | success                                              |
//! | 1    | the command ran but failed (build error, bad diff…) |
//! | 2    | preconditions not met (not a Comline project, …)    |
//! |      | (clap also exits 2 for usage errors)                |
//!
//! `comline store migrate --check` has codes of its own, since it reports
//! rather than fails (see [`CliError::Exit`]): 0 nothing to migrate, 2 a
//! migration would run, 3 it would be refused.

use std::path::PathBuf;

use comline_core::package::build::cas::migrate::MigrateError;
use comline_core::package::build::cas::object_store::StoreError;
use miette::Diagnostic;
use thiserror::Error;

/// The command ran but failed (build error, unresolved diff ref, …).
pub const EXIT_FAILURE: i32 = 1;
/// A precondition was not met (not a Comline project, nothing built yet, …).
pub const EXIT_PRECONDITION: i32 = 2;

/// Errors the CLI raises itself (as opposed to errors bubbled up from
/// `comline-core`). Kept small on purpose — most failures are just
/// `miette::miette!(...)`.
#[derive(Debug, Error, Diagnostic)]
pub enum CliError {
    #[error("`{}` is not a Comline project (no config.idp)", .0.display())]
    #[diagnostic(help("run `comline new <name>` to scaffold one, or pass `--path <dir>`"))]
    NotAProject(PathBuf),

    #[error("no builds found in `{}` — run `comline build` first", .0.display())]
    NothingBuilt(PathBuf),

    /// The store in `.comline/` can't be used by this comline: another IR
    /// format, no stamp, a stamp that isn't one, or a migration or reset
    /// that was interrupted. Core's own message, which says nothing was
    /// changed, and names the command that fixes it where there is one.
    /// No command changes the store to get past this: only the user's
    /// `comline store migrate` and `comline store reset` do.
    #[error("{0}")]
    #[diagnostic(help(
        "no command changes `.comline/` to get past this. `comline store status` says what \
         there is; `comline store migrate` (`--check` shows what it would do first) migrates \
         it, and `comline store reset` moves it aside to start a new history, keeping \
         `.comline/deps-cache/` and deleting nothing"
    ))]
    Store(StoreError),

    /// The engine refused a `comline store` command, and said why in its own
    /// words: nothing to reset, a store that is never reset, a prune with
    /// nothing verified to replace the backups, an option that doesn't apply,
    /// an earlier attempt still on disk. Nothing was changed.
    #[error("{0}")]
    StoreRefused(MigrateError),

    /// Not a failure to print: the exit code `comline store migrate --check`
    /// reports (what it found was already printed).
    #[error("exit {0}")]
    Exit(i32),

    #[error("refusing to reset the version history without confirmation")]
    #[diagnostic(help(
        "re-run in a terminal to be asked, or pass `--force`. Nothing is deleted either way: \
         the store is moved aside until `comline store prune`"
    ))]
    ResetConfirmationRequired,

    #[error("refusing to delete the version history without confirmation")]
    #[diagnostic(help("re-run in a terminal to confirm, or pass `--force`"))]
    ConfirmationRequired,

    #[error(
        "refusing to accept unverified protocols without confirmation: re-run in a terminal \
         to be asked, or pass `{flag}` to accept them"
    )]
    AcceptanceRequired { flag: &'static str },

    #[error(
        "refusing to {what} without confirmation: re-run in a terminal to be asked, or pass \
         `--yes`"
    )]
    ReleaseConfirmationRequired { what: &'static str },
}

/// A `comline-core` error as the CLI reports it: a store core refused
/// ([`StoreError`], which core carries in the `eyre` report) becomes
/// [`CliError::Store`], and an engine refusal that is a precondition
/// ([`MigrateError`]: nothing to reset, a store that is never reset, a prune
/// refused) becomes [`CliError::StoreRefused`], both exit 2 with core's
/// message as it is; anything else is `context` plus the detail, as
/// [`crate::commands::core_err`] has it.
pub fn from_core(context: &str, e: eyre::Report) -> miette::Report {
    if let Some(store) = e.downcast_ref::<StoreError>() {
        return CliError::Store(store.clone()).into();
    }
    match e.downcast_ref::<MigrateError>() {
        Some(
            refused @ (MigrateError::NothingToReset
            | MigrateError::ResetRefused { .. }
            | MigrateError::PruneRefused { .. }
            | MigrateError::Assume { .. }
            | MigrateError::Leftover { .. }),
        ) => CliError::StoreRefused(refused.clone()).into(),
        _ => crate::commands::core_err(context, e),
    }
}

/// Map a finished-command error to its process exit code.
pub fn exit_code_for(report: &miette::Report) -> i32 {
    match report.downcast_ref::<CliError>() {
        Some(CliError::NotAProject(_)) => EXIT_PRECONDITION,
        Some(CliError::NothingBuilt(_)) => EXIT_PRECONDITION,
        Some(CliError::Store(_)) => EXIT_PRECONDITION,
        Some(CliError::StoreRefused(_)) => EXIT_PRECONDITION,
        Some(CliError::Exit(code)) => *code,
        Some(CliError::ResetConfirmationRequired) => EXIT_PRECONDITION,
        Some(CliError::ConfirmationRequired) => EXIT_PRECONDITION,
        Some(CliError::AcceptanceRequired { .. }) => EXIT_PRECONDITION,
        Some(CliError::ReleaseConfirmationRequired { .. }) => EXIT_PRECONDITION,
        None => EXIT_FAILURE,
    }
}
