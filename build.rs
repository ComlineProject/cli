//! Generates man pages and shell completions into `OUT_DIR` at build time so
//! packagers can install them. End users can also get completions at runtime via
//! `comline completions <shell>`.
//!
//! `src/cli.rs` is intentionally dependency-light so it can be shared here.

use std::env;
use std::fs;
use std::io::Error;
use std::path::{Path, PathBuf};

use clap::{Command, CommandFactory};
use clap_complete::Shell;

#[path = "src/cli.rs"]
mod cli;

fn main() -> Result<(), Error> {
    println!("cargo:rerun-if-changed=src/cli.rs");
    println!("cargo:rerun-if-changed=build.rs");

    let Some(out_dir) = env::var_os("OUT_DIR").map(PathBuf::from) else {
        return Ok(());
    };

    let man_dir = out_dir.join("man");
    fs::create_dir_all(&man_dir)?;
    render_man_pages(&man_dir, &cli::Cli::command())?;

    let completions_dir = out_dir.join("completions");
    fs::create_dir_all(&completions_dir)?;
    let mut command = cli::Cli::command();
    for shell in [
        Shell::Bash,
        Shell::Zsh,
        Shell::Fish,
        Shell::PowerShell,
        Shell::Elvish,
    ] {
        clap_complete::generate_to(shell, &mut command, "comline", &completions_dir)?;
    }

    Ok(())
}

fn render_man_pages(dir: &Path, command: &Command) -> Result<(), Error> {
    let name = command.get_name().to_string();
    write_man(&dir.join(format!("{name}.1")), command.clone(), &name)?;

    for sub in command.get_subcommands() {
        if sub.get_name() == "help" {
            continue;
        }
        let page = format!("{name}-{}", sub.get_name());
        write_man(&dir.join(format!("{page}.1")), sub.clone(), &page)?;
    }
    Ok(())
}

fn write_man(path: &Path, command: Command, title: &str) -> Result<(), Error> {
    let mut buffer = Vec::new();
    clap_mangen::Man::new(command)
        .title(title.to_uppercase())
        .render(&mut buffer)?;
    fs::write(path, buffer)
}
