//! Command-line surface for `comline`.
//!
//! This module is intentionally self-contained (only `clap`, `clap_complete` and
//! `std`) so that `build.rs` can `#[path]`-include it to generate man pages and
//! shell completions from the exact same definitions the binary uses.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

#[derive(Parser, Debug)]
#[command(
    name = "comline",
    version,
    about = "Build, validate, diff and generate code from Comline schemas",
    long_about = None,
)]
pub struct Cli {
    /// Run against this directory instead of the current one
    #[arg(short, long, global = true, value_name = "DIR")]
    pub path: Option<PathBuf>,

    /// Increase log verbosity (-v: debug, -vv: trace); surfaces comline-core diagnostics
    #[arg(
        short,
        long,
        global = true,
        action = clap::ArgAction::Count,
        conflicts_with = "quiet",
    )]
    pub verbose: u8,

    /// Silence progress output; only errors are printed
    #[arg(short, long, global = true)]
    pub quiet: bool,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Compile the project and freeze a new immutable version
    #[command(long_about = "\
Compile and validate every schema, then freeze the result into the project's \
content-addressable store (`.comline/`). The project version is bumped \
automatically to match the largest schema change since the last build \
(major for breaking changes, minor for new features, patch for tweaks).

Examples:
  comline build
  comline build --watch")]
    Build {
        /// Enable release optimizations (reserved; currently a no-op)
        #[arg(short, long)]
        release: bool,

        /// Rebuild automatically when a schema or `config.idp` changes
        #[arg(short, long)]
        watch: bool,
    },

    /// Validate the project without freezing a version or writing artifacts
    #[command(long_about = "\
Parse, resolve and validate every schema and the project config, reporting the \
first error found. Unlike `build`, this never writes to `.comline/` and never \
bumps the version, so it is safe to run in editors and pre-commit hooks.")]
    Check,

    /// Compile the project and write generated code for each configured target
    #[command(long_about = "\
Build the project, then run every code generator configured in `config.idp`, \
writing one file per schema namespace.

Examples:
  comline generate
  comline generate --target rust
  comline generate --watch")]
    Generate {
        /// Only generate for this language (e.g. `rust`); default: every configured target
        #[arg(short, long, value_name = "LANG")]
        target: Option<String>,

        /// Regenerate automatically when a schema or `config.idp` changes
        #[arg(short, long)]
        watch: bool,
    },

    /// Show the schema changes between two built versions
    #[command(long_about = "\
Compare two frozen versions from the project's `.comline/` store and print the \
breaking changes, new features and modifications between them — the same \
report `build` shows, on demand.

Each argument is a version string (`0.2.0`), a commit hash, or `HEAD`.

Examples:
  comline diff 0.1.0 0.2.0
  comline diff 0.1.0            # compares 0.1.0 against HEAD")]
    Diff {
        /// Base version: a version string, a commit hash, or `HEAD`
        #[arg(value_name = "OLD")]
        old: String,

        /// Target version: a version string, a commit hash, or `HEAD`
        #[arg(value_name = "NEW", default_value = "HEAD")]
        new: String,
    },

    /// Remove build artifacts: the `.comline/` store and generated files
    #[command(long_about = "\
Delete the project's `.comline/` content-addressable store and any files left \
by `generate`. The next `build` starts a fresh version history at 0.0.1.")]
    Clean {
        /// List what would be removed without deleting anything
        #[arg(long)]
        dry_run: bool,
    },

    /// Create a new Comline project in a new directory
    New {
        /// Project name; also the directory that gets created
        name: String,

        /// Also run `git init` inside the new project
        #[arg(long)]
        git: bool,
    },

    /// Print a shell completion script to stdout
    #[command(long_about = "\
Write a completion script for the given shell to stdout.

Examples:
  comline completions bash > /etc/bash_completion.d/comline
  comline completions fish > ~/.config/fish/completions/comline.fish")]
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },
}
