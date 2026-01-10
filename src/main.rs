mod commands;

use clap::Parser;
use commands::{Cli, Commands};
use miette::{Context, IntoDiagnostic, Result};
use std::env;
use std::fs;
use std::path::Path;
use tracing::{error, info, warn};
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
                Ok(_) => {
                    info!("Project built successfully!");
                    // Code generation logic has been moved to `generate` command
                }
                Err(e) => {
                    error!("Build failed: {:?}", e);
                    return Err(miette::miette!("Build failed: {:?}", e));
                }
            }
        }
        Commands::Generate => {
             info!("Generating code...");
             let current_dir = env::current_dir()
                .into_diagnostic()
                .wrap_err("Failed to get current directory")?;

            if !comline_core::package::config::is_package_path(&current_dir) {
                miette::bail!("Current directory is not a Comline project (missing config.idp)");
            }

            match comline_core::package::build::build(&current_dir) {
                Ok(ctx) => {
                     if let Some(frozen_config) = &ctx.config_frozen {
                        for unit in frozen_config {
                            if let comline_core::package::config::ir::frozen::FrozenUnit::CodeGeneration(gen) = unit {
                                let lang_name_ver = &gen.name;
                                let parts: Vec<&str> = lang_name_ver.split('#').collect();
                                if parts.len() != 2 {
                                    warn!("Skipping invalid language specifier: {}", lang_name_ver);
                                    continue;
                                }
                                let lang = parts[0];
                                let version = parts[1];

                                if let Some((generator, ext)) = comline_core::codelib_gen::find_generator(lang, version) {
                                    info!("Generating {} code...", lang);
                                    for schema_ctx in &ctx.schema_contexts {
                                        let schema_ctx = schema_ctx.borrow();
                                        // Clone the units to avoid borrowing issues with RefCell
                                        let frozen_units_opt = schema_ctx.frozen_schema.borrow().clone();
                                        
                                        if let Some(frozen_units) = frozen_units_opt {
                                            let output = generator(&frozen_units);
                                            // Assuming simple file naming strategy for now
                                            let file_name = format!("{}.{}", schema_ctx.namespace_joined(), ext);
                                            let file_path = current_dir.join(&file_name);
                                            if let Err(e) = fs::write(&file_path, output) {
                                                    error!("Failed to write generated file {}: {:?}", file_name, e);
                                            } else {
                                                    info!("Generated {}", file_name);
                                            }
                                        }
                                    }
                                } else {
                                    warn!("No generator found for {} version {}", lang, version);
                                }
                            }
                        }
                    }
                    info!("Code generation complete!");
                }
                Err(e) => {
                    error!("Code generation failed: {:?}", e);
                    return Err(miette::miette!("Code generation failed: {:?}", e));
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
