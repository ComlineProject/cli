//! `comline completions <shell>` — print a completion script to stdout.

use clap::CommandFactory;
use clap_complete::{generate, Shell};
use miette::Result;

use crate::cli::Cli;

pub fn run(shell: Shell) -> Result<()> {
    let mut command = Cli::command();
    let bin_name = command.get_name().to_string();
    generate(shell, &mut command, bin_name, &mut std::io::stdout());
    Ok(())
}
