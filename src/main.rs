mod changes;
mod cli;
mod commands;
mod error;
mod gen_config;
mod ui;
mod watch;

use std::env;
use std::process::ExitCode;

use clap::Parser;
use miette::Result;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use cli::{Cli, Commands};

fn main() -> ExitCode {
    reset_sigpipe();

    let cli = Cli::parse();

    init_tracing(&cli);
    ui::set_quiet(cli.quiet);
    ui::set_verbose(cli.verbose > 0);
    ui::set_plain(cli.plain);

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(report) => {
            let code = error::exit_code_for(&report);
            ui::error(format!("{report}"));
            for cause in report.chain().skip(1) {
                ui::error(format!("  caused by: {cause}"));
            }
            ExitCode::from(code as u8)
        }
    }
}

/// `tracing` carries `comline-core` diagnostics only; the CLI's own output goes
/// through [`ui`]. Everything is written to stderr so stdout stays a clean
/// channel for payloads (`comline completions`). Verbosity is shifted down a
/// notch from the usual so the default run is quiet: `-v` shows info, `-vv`
/// debug, `-vvv` trace. `RUST_LOG` still overrides.
fn init_tracing(cli: &Cli) {
    let level = if cli.quiet {
        "error"
    } else {
        match cli.verbose {
            0 => "warn",
            1 => "info",
            2 => "debug",
            _ => "trace",
        }
    };

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            env::var("RUST_LOG")
                .unwrap_or_else(|_| format!("comline={level},comline_core={level}")),
        ))
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .init();
}

/// Rust ignores `SIGPIPE`, so writing to a closed pipe (`comline completions
/// fish | head`) surfaces as an `EPIPE` write error that downstream libraries
/// `.expect()` into a panic. Restore the default disposition so the process just
/// exits quietly instead, like every other Unix CLI.
#[cfg(unix)]
fn reset_sigpipe() {
    // SAFETY: called once at startup, before any other threads exist.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn reset_sigpipe() {}

fn run(cli: Cli) -> Result<()> {
    let work_dir = match cli.path {
        Some(path) => path,
        None => env::current_dir()
            .map_err(|e| miette::miette!("failed to determine the current directory: {e}"))?,
    };

    match cli.command {
        Commands::Build { release, watch } => commands::build::run(&work_dir, release, watch),
        Commands::Check => commands::check::run(&work_dir),
        Commands::Generate {
            target,
            out,
            layout,
            mode,
            watch,
        } => {
            let overrides = gen_config::Overrides {
                target: target.as_deref(),
                out: out.as_deref().map(|p| p.to_str().unwrap_or_default()),
                layout: layout.as_deref(),
                mode: mode.as_deref(),
            };
            commands::generate::run(&work_dir, &overrides, watch)
        }
        Commands::Diff { old, new } => commands::diff::run(&work_dir, &old, &new),
        Commands::Clean { dry_run } => commands::clean::run(&work_dir, dry_run),
        Commands::New { name, git } => commands::new::run(&work_dir, &name, git),
        Commands::Completions { shell } => commands::completions::run(shell),
    }
}
