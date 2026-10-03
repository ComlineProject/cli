mod changes;
mod cli;
mod commands;
mod error;
mod gen_config;
mod history;
mod ui;
mod watch;

use std::env;
use std::process::ExitCode;

use clap::{CommandFactory, FromArgMatches};
use miette::Result;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use cli::{Cli, Commands};

fn main() -> ExitCode {
    reset_sigpipe();

    let cli = parse_cli();

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

/// Parse CLI args with an enriched `--version` (`-V` stays the bare
/// `CARGO_PKG_VERSION` `clap`'s derive attribute already gives `Cli`).
/// Built as a `Command`, not the `#[command(long_version = ...)]` derive
/// attribute, because `build.rs` also compiles `cli.rs` (via `#[path]`, for
/// man-page/completion generation) *before* it has emitted these env vars
/// for the real binary — a `const` baked in by that attribute would need
/// `env!()`, which hard-fails the build in that earlier context. Building
/// the string here instead, with `option_env!()`, means every field just
/// degrades to `"unknown"` if `build.rs`'s own `git`/`date` capture
/// (`emit_version_info`) came up empty, rather than failing anything.
fn parse_cli() -> Cli {
    let long_version = format!(
        "{}\ncommit-hash: {}\ncommit-branch: {}\ncommit-count: {}\nbuild-date: {}",
        env!("CARGO_PKG_VERSION"),
        option_env!("COMLINE_GIT_HASH").unwrap_or("unknown"),
        option_env!("COMLINE_GIT_BRANCH").unwrap_or("unknown"),
        option_env!("COMLINE_COMMIT_COUNT").unwrap_or("unknown"),
        option_env!("COMLINE_BUILD_DATE").unwrap_or("unknown"),
    );
    // `Command::long_version` needs a `&'static str`, not an owned
    // `String` - leaking it is fine here, it lives for the process anyway.
    let long_version: &'static str = Box::leak(long_version.into_boxed_str());
    let matches = Cli::command().long_version(long_version).get_matches();
    Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit())
}

/// `tracing` carries `comline-core` diagnostics only; the CLI's own output goes
/// through [`ui`]. Everything is written to stderr so stdout stays a clean
/// channel for payloads (`comline completions`, `comline targets`). Verbosity
/// is shifted down a notch from the usual so the default run is quiet: `-v`
/// shows info, `-vv` debug, `-vvv` trace. `RUST_LOG` still overrides.
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
        Commands::Reset { force, dry_run } => commands::reset::run(&work_dir, force, dry_run),
        Commands::New { name, git } => commands::new::run(&work_dir, &name, git),
        Commands::Completions { shell } => commands::completions::run(shell),
        Commands::Targets => commands::targets::run(),
    }
}
