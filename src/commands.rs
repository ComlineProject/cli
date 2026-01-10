use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "comline", version, about = "Comline CLI", long_about = None)]
pub struct Cli {
    /// Optional working directory
    #[arg(short, long, global = true)]
    pub path: Option<PathBuf>,

    /// Enable verbose output (can be used multiple times: -v, -vv, -vvv)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    pub verbose: u8,

    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Build the current project
    Build {
        /// Enable release optimizations
        #[arg(short, long)]
        release: bool,
    },
    /// Check the current project for errors
    Check,
    /// Compiles the current project and generates code
    Generate,
    /// Create a new Comline project
    New {
        /// The name of the project
        name: String,
    },
}
