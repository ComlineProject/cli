//! Generates man pages and shell completions into `OUT_DIR` at build time so
//! packagers can install them. End users can also get completions at runtime via
//! `comline completions <shell>`.
//!
//! `src/cli.rs` is intentionally dependency-light so it can be shared here.

use std::env;
use std::fs;
use std::io::Error;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use clap::{Command, CommandFactory};
use clap_complete::Shell;

#[path = "src/cli.rs"]
mod cli;

fn main() -> Result<(), Error> {
    println!("cargo:rerun-if-changed=src/cli.rs");
    println!("cargo:rerun-if-changed=build.rs");
    emit_version_info();

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

/// Sets `COMLINE_GIT_HASH` / `COMLINE_GIT_BRANCH` / `COMLINE_COMMIT_COUNT` /
/// `COMLINE_BUILD_DATE` for `cli.rs`'s `LONG_VERSION` to pull in via
/// `env!()`. Every one of these degrades to `"unknown"` rather than
/// failing the build — a source tarball with no `.git` directory, or a
/// machine with no `git`/`date` on PATH, still produces a working binary.
///
/// Re-run (and so re-captured) whenever the current commit or branch
/// changes; a build with no new commits reuses the cached binary anyway, so
/// there's nothing to refresh in that case.
fn emit_version_info() {
    println!("cargo:rerun-if-changed=.git/HEAD");
    println!("cargo:rerun-if-changed=.git/refs");

    let hash =
        run(&["git", "rev-parse", "--short=12", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let dirty = run(&["git", "status", "--porcelain"]).is_some_and(|s| !s.is_empty());
    let branch =
        run(&["git", "rev-parse", "--abbrev-ref", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let commit_count =
        run(&["git", "rev-list", "--count", "HEAD"]).unwrap_or_else(|| "unknown".to_string());
    let build_date =
        run(&["date", "-u", "+%Y-%m-%dT%H:%M:%SZ"]).unwrap_or_else(|| "unknown".to_string());

    println!(
        "cargo:rustc-env=COMLINE_GIT_HASH={}{}",
        hash,
        if dirty { "-dirty" } else { "" }
    );
    println!("cargo:rustc-env=COMLINE_GIT_BRANCH={branch}");
    println!("cargo:rustc-env=COMLINE_COMMIT_COUNT={commit_count}");
    println!("cargo:rustc-env=COMLINE_BUILD_DATE={build_date}");
}

/// Run a command, returning trimmed stdout on success - `None` on any
/// failure (missing binary, non-zero exit, not a git repo, ...).
fn run(args: &[&str]) -> Option<String> {
    let output = ProcessCommand::new(args[0])
        .args(&args[1..])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_string())
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
