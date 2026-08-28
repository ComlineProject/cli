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
}

/// Map a finished-command error to its process exit code.
pub fn exit_code_for(report: &miette::Report) -> i32 {
    match report.downcast_ref::<CliError>() {
        Some(CliError::NotAProject(_)) => EXIT_PRECONDITION,
        Some(CliError::NothingBuilt(_)) => EXIT_PRECONDITION,
        None => EXIT_FAILURE,
    }
}
