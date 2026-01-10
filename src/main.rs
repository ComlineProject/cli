mod commands;

use clap::Parser;
use commands::{Cli, Commands};
use miette::{Context, IntoDiagnostic, Result};
use std::env;
use std::fs;
use std::path::Path;
use tracing::{error, info};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    match &cli.command {
        Commands::Build { release } => {
            info!("Building project... Release mode: {}", release);
            let current_dir = env::current_dir()
                .into_diagnostic()
                .wrap_err("Failed to get current directory")?;

            if !comline_core::package::config::is_package_path(&current_dir) {
                miette::bail!("Current directory is not a Comline project (missing config.idp)");
            }

            match comline_core::package::build::build(&current_dir) {
                Ok(_ctx) => {
                    info!("Project built successfully!");
                    // TODO: Implement code generation output if not handled by core
                }
                Err(e) => {
                    error!("Build failed: {:?}", e);
                    return Err(miette::miette!("Build failed: {:?}", e));
                }
            }
        }
        Commands::Check => {
            info!("Checking project...");
            let current_dir = env::current_dir()
                .into_diagnostic()
                .wrap_err("Failed to get current directory")?;

            if !comline_core::package::config::is_package_path(&current_dir) {
                miette::bail!("Current directory is not a Comline project (missing config.idp)");
            }

            // Using build() for check as well for now as it performs full validation
            match comline_core::package::build::build(&current_dir) {
                Ok(_) => info!("Check passed!"),
                Err(e) => {
                    error!("Check failed: {:?}", e);
                    return Err(miette::miette!("Check failed: {:?}", e));
                }
            }
        }
        Commands::New { name } => {
            info!("Creating new project: {}", name);
            let path = Path::new(name);
            if path.exists() {
                miette::bail!("Directory '{}' already exists", name);
            }
            fs::create_dir_all(path)
                .into_diagnostic()
                .wrap_err("Failed to create project directory")?;

            let config_path = path.join("config.idp");
            let config_content = format!("congregation {}\nspecification_version = 1\n", name);
            fs::write(&config_path, config_content)
                .into_diagnostic()
                .wrap_err("Failed to write config.idp")?;

            info!("Created new project at {}", path.display());
        }
    }

    Ok(())
}
