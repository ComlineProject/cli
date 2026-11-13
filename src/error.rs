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

use std::path::PathBuf;

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
    /// format, no stamp, or a stamp that isn't one. Core's own message,
    /// which says nothing was changed. No command changes the store to
    /// resolve it; only an explicit `comline reset` deletes it.
    #[error("{0}")]
    #[diagnostic(help(
        "no command changes `.comline/` to get past this. If you don't need the history, \
         `comline reset` deletes it (after asking, and keeping `.comline/deps-cache/`)"
    ))]
    Store(StoreError),

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
/// [`CliError::Store`], a precondition (exit 2) with core's message
/// as it is; anything else is `context` plus the detail, as
/// [`crate::commands::core_err`] has it.
pub fn from_core(context: &str, e: eyre::Report) -> miette::Report {
    match e.downcast_ref::<StoreError>() {
        Some(store) => CliError::Store(store.clone()).into(),
        None => crate::commands::core_err(context, e),
    }
}

/// Map a finished-command error to its process exit code.
pub fn exit_code_for(report: &miette::Report) -> i32 {
    match report.downcast_ref::<CliError>() {
        Some(CliError::NotAProject(_)) => EXIT_PRECONDITION,
        Some(CliError::NothingBuilt(_)) => EXIT_PRECONDITION,
        Some(CliError::Store(_)) => EXIT_PRECONDITION,
        Some(CliError::ConfirmationRequired) => EXIT_PRECONDITION,
        Some(CliError::AcceptanceRequired { .. }) => EXIT_PRECONDITION,
        Some(CliError::ReleaseConfirmationRequired { .. }) => EXIT_PRECONDITION,
        None => EXIT_FAILURE,
    }
}
