//! One module per subcommand. Each exposes a `run(...)` that returns
//! `miette::Result<()>`; `main` maps the error to an exit code.

pub mod add;
pub mod build;
pub mod check;
pub mod clean;
pub mod completions;
pub mod diff;
pub mod generate;
pub mod new;
pub mod reset;
pub mod targets;

use std::path::Path;

use miette::Result;

use crate::error::CliError;

/// Fail with [`CliError::NotAProject`] unless `dir` contains a `config.idp`.
pub fn ensure_project(dir: &Path) -> Result<()> {
    if comline_core::package::config::is_package_path(dir) {
        Ok(())
    } else {
        Err(CliError::NotAProject(dir.to_path_buf()).into())
    }
}

/// Wrap a `comline-core` (`eyre`) error as a `miette` one for the reporting
/// path. `{:#}` renders the message plus its cause chain, without a
/// backtrace. `comline-core`'s own diagnostics (parse/validation errors)
/// now arrive as a multi-line, already-rendered `ariadne` source snippet
/// (`comline_core::diagnostics::render`) - gluing `"{context}: "` onto the
/// front of that would run the prefix into the snippet's own box-drawing on
/// one line, so a multi-line detail gets its own paragraph instead.
pub fn core_err(context: &str, e: impl std::fmt::Display) -> miette::Report {
    let detail = format!("{e:#}");
    if detail.contains('\n') {
        miette::miette!("{context}:\n\n{detail}")
    } else {
        miette::miette!("{context}: {detail}")
    }
}
