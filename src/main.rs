mod commands;

use clap::Parser;
use commands::{Cli, Commands};
use miette::{Context, IntoDiagnostic, Result};
use std::env;
use std::fs;
use tracing::{error, info, warn};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

/// Display schema changes in a user-friendly format
fn display_schema_changes(changes: &comline_core::schema::ir::diff::SchemaChanges) {
    use comline_core::schema::ir::diff::{BreakingChange, NewFeature, Modification};
    
    // Display breaking changes
    if !changes.breaking_changes.is_empty() {
        warn!("🔴 Breaking changes ({}):", changes.breaking_changes.len());
        for change in &changes.breaking_changes {
            match change {
                BreakingChange::RemovedStruct { name } => {
                    warn!("  ❌ Removed struct `{}`", name);
                }
                BreakingChange::RemovedEnum { name } => {
                    warn!("  ❌ Removed enum `{}`", name);
                }
                BreakingChange::RemovedField { type_name, field_name } => {
                    warn!("  ❌ Removed field `{}.{}`", type_name, field_name);
                }
                BreakingChange::ChangedFieldType { type_name, field_name, old_type, new_type } => {
                    warn!("  🔄 Changed `{}.{}`: {} → {}", type_name, field_name, old_type, new_type);
                }
                BreakingChange::RemovedEnumVariant { enum_name, variant } => {
                    warn!("  ❌ Removed `{}::{}`", enum_name, variant);
                }
                BreakingChange::RemovedFunction { protocol_name, function_name } => {
                    warn!("  ❌ Removed function `{}.{}`", protocol_name, function_name);
                }
                BreakingChange::ChangedFunctionSignature { protocol_name, function_name, details } => {
                    warn!("  🔄 Changed signature of `{}.{}`: {}", protocol_name, function_name, details);
                }
                BreakingChange::RemovedProtocol { name } => {
                    warn!("  ❌ Removed protocol `{}`", name);
                }
            }
        }
    }
    
    // Display new features
    if !changes.new_features.is_empty() {
        info!("🟢 New features ({}):", changes.new_features.len());
        for feature in &changes.new_features {
            match feature {
                NewFeature::AddedStruct { name, field_count } => {
                    info!("  ➕ Added struct `{}` ({} fields)", name, field_count);
                }
                NewFeature::AddedEnum { name, variant_count } => {
                    info!("  ➕ Added enum `{}` ({} variants)", name, variant_count);
                }
                NewFeature::AddedField { type_name, field_name, field_type, optional } => {
                    let opt_marker = if *optional { " (optional)" } else { "" };
                    info!("  ➕ Added field `{}.{}`: {}{}", type_name, field_name, field_type, opt_marker);
                }
                NewFeature::AddedEnumVariant { enum_name, variant } => {
                    info!("  ➕ Added variant `{}::{}`", enum_name, variant);
                }
                NewFeature::AddedFunction { protocol_name, function_name, signature } => {
                    info!("  ➕ Added function `{}.{}`: {}", protocol_name, function_name, signature);
                }
                NewFeature::AddedProtocol { name, function_count } => {
                    info!("  ➕ Added protocol `{}` ({} functions)", name, function_count);
                }
            }
        }
    }
    
    // Display modifications
    if !changes.modifications.is_empty() {
        info!("🔵 Modifications ({}):", changes.modifications.len());
        for modification in &changes.modifications {
            match modification {
                Modification::FieldMadeOptional { type_name, field_name } => {
                    info!("  🔧 Made field `{}.{}` optional", type_name, field_name);
                }
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::new(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "info".into()),
        ))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let cli = Cli::parse();

    let work_dir = match cli.path {
        Some(p) => p,
        None => env::current_dir()
            .into_diagnostic()
            .wrap_err("Failed to get current directory")?,
    };

    match &cli.command {
        Commands::Build { release } => {
            info!("Building project... Release mode: {}", release);
            
            if !comline_core::package::config::is_package_path(&work_dir) {
                miette::bail!("Current directory is not a Comline project (missing config.idp)");
            }

            match comline_core::package::build::build(&work_dir) {
                Ok(build_result) => {
                    info!("✅ Project built successfully!");
                    
                    // Display version information only if there are changes
                    if build_result.is_initial_build() {
                        info!("📦 Initial version: {}", build_result.current_version);
                    } else {
                        // Check if there are actual schema changes
                        let has_changes = build_result.schema_changes
                            .as_ref()
                            .map(|changes| !changes.is_empty())
                            .unwrap_or(false);
                        
                        if has_changes {
                            if let Some(version_change) = build_result.version_change() {
                                info!("📦 Version: {}", version_change);
                            }
                            
                            // Display schema changes
                            if let Some(changes) = &build_result.schema_changes {
                                display_schema_changes(changes);
                                
                                // Display version bump type
                                match build_result.version_bump {
                                    comline_core::package::build::VersionBump::Major => {
                                        warn!("⬆️  Major version bump applied (breaking changes)");
                                    },
                                    comline_core::package::build::VersionBump::Minor => {
                                        info!("⬆️  Minor version bump applied (new features)");
                                    },
                                    comline_core::package::build::VersionBump::Patch => {
                                        info!("⬆️  Patch version bump applied (modifications)");
                                    },
                                    comline_core::package::build::VersionBump::None => {
                                        info!("No version bump (no changes)");
                                    }
                                }
                            }
                        } else {
                            info!("📦 Version: {} (no changes)", build_result.current_version);
                        }
                    }
                }
                Err(e) => {
                    error!("Build failed: {:?}", e);
                    return Err(miette::miette!("Build failed: {:?}", e));
                }
            }
        }
        Commands::Generate => {
             info!("Generating code...");

            if !comline_core::package::config::is_package_path(&work_dir) {
                miette::bail!("Current directory is not a Comline project (missing config.idp)");
            }

            match comline_core::package::build::build(&work_dir) {
                Ok(build_result) => {
                     if let Some(frozen_config) = &build_result.context.config_frozen {
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
                                    for schema_ctx in &build_result.context.schema_contexts {
                                        let schema_ctx = schema_ctx.borrow();
                                        // Clone the units to avoid borrowing issues with RefCell
                                        let frozen_units_opt = schema_ctx.frozen_schema.borrow().clone();
                                        
                                        if let Some(frozen_units) = frozen_units_opt {
                                            let output = generator(&frozen_units);
                                            // Assuming simple file naming strategy for now
                                            let file_name = format!("{}.{}", schema_ctx.namespace_joined(), ext);
                                            let file_path = work_dir.join(&file_name);
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

            if !comline_core::package::config::is_package_path(&work_dir) {
                miette::bail!("Current directory is not a Comline project (missing config.idp)");
            }

            // Using build() for check as well for now as it performs full validation
            match comline_core::package::build::build(&work_dir) {
                Ok(_) => info!("Check passed!"),
                Err(e) => {
                    error!("Check failed: {:?}", e);
                    return Err(miette::miette!("Check failed: {:?}", e));
                }
            }
        }
        Commands::New { name } => {
            info!("Creating new project: {}", name);
            let path = work_dir.join(name);
            if path.exists() {
                miette::bail!("Directory '{}' already exists", name);
            }
            fs::create_dir_all(&path)
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
