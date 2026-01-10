use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "comline", version, about = "Comline CLI", long_about = None)]
pub struct Cli {
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
