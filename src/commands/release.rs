//! `comline release` — turn the builds since the last release into a
//! version.
//!
//! Builds record history under pre-release versions (`0.3.0-dev.2`); a
//! release is the version consumers see. It builds first, so what's released
//! is what's in `src/` now, then takes the highest bump the changes since the
//! last release call for. `--raise` goes higher, with a reason. `release 1.0`
//! moves a `0.x` package to `1.0.0` after saying what that commits to.

use std::io::{self, IsTerminal, Write};
use std::path::Path;

use comline_core::package::build::{self, ReleaseRequest, VersionBump};
use comline_core::package::config::ir::context::ProjectContext;
use comline_core::schema::ir::frozen::unit::FrozenUnit;
use comline_core::schema::ir::unverified;
use miette::{IntoDiagnostic, Result};

use crate::changes;
use crate::cli::ReleaseLevel as Level;
use crate::commands::{core_err, ensure_project};
use crate::error::CliError;
use crate::ui;

fn bump(level: Level) -> VersionBump {
    match level {
        Level::Patch => VersionBump::Patch,
        Level::Minor => VersionBump::Minor,
        Level::Major => VersionBump::Major,
    }
}

pub fn run(
    work_dir: &Path,
    version: Option<&str>,
    raise: Option<Level>,
    reason: Option<String>,
    yes: bool,
) -> Result<()> {
    ensure_project(work_dir)?;
    let stable = match version {
        None => false,
        Some("1.0" | "1.0.0") => true,
        Some(other) => {
            return Err(miette::miette!(
                "`comline release {other}`: the version comes from the changes; the only one \
                 you name is `1.0`, to make the package stable"
            ))
        }
    };
    if raise.is_some() && reason.as_deref().map_or(true, |r| r.trim().is_empty()) {
        return Err(miette::miette!(
            "--raise needs --reason: it's recorded with the release and shown in its history"
        ));
    }

    ui::step(format!("Building project{}", ui::at_path(work_dir)));
    let spinner = ui::spinner("compiling schemas");
    let built = build::build(work_dir);
    ui::finish_spinner(spinner);
    let built = built.map_err(|e| core_err("build failed", e))?;
    for warning in built.context.rendered_warnings() {
        ui::warn(warning);
    }

    if stable {
        explain_stable(&built.context, built.last_release.as_deref());
        if !confirm("Release 1.0.0", yes, "make the package stable")? {
            ui::note("Not released");
            return Ok(());
        }
    } else if matches!(raise, Some(Level::Major))
        && !confirm(
            "Raise to a major release",
            yes,
            "raise the release to major",
        )?
    {
        ui::note("Not released");
        return Ok(());
    }

    let request = ReleaseRequest {
        raise: raise.map(bump),
        reason,
        stable,
    };
    let released =
        build::release(work_dir, &request).map_err(|e| core_err("couldn't release", e))?;

    ui::success(format!("Released {}", released.version));
    match &released.previous_release {
        Some(previous) => ui::detail(format!("{previous} -> {}", released.version)),
        None => ui::detail("the first release".to_string()),
    }
    if let Some(changes) = &released.changes {
        changes::render(&changes.schema_changes);
        for dependency in &changes.dependency_changes {
            ui::group(format!("In dependency `{}`:", dependency.name));
            changes::render(&dependency.changes);
        }
    }
    let record = &released.record;
    if record.stable {
        ui::detail("stable: from now on a breaking change is a major release".to_string());
    } else if released.previous_release.is_some() {
        ui::detail(format!(
            "{} bump{}",
            describe(record.applied),
            match (record.computed == record.applied, record.covered) {
                (true, true) => " (the raise was already covered by the changes)".to_string(),
                (true, false) => String::new(),
                (false, _) => format!(", raised from {}", describe(record.computed)),
            }
        ));
    }
    if let Some(reason) = &record.reason {
        ui::detail(format!("reason: {reason}"));
    }
    Ok(())
}

fn describe(bump: VersionBump) -> &'static str {
    match bump {
        VersionBump::Major => "major",
        VersionBump::Minor => "minor",
        VersionBump::Patch => "patch",
        VersionBump::None => "no",
    }
}

/// What 1.0 commits to, and what the package exposes. The things worth
/// settling first are listed for information, not required.
fn explain_stable(context: &ProjectContext, last_release: Option<&str>) {
    ui::warn(format!(
        "Releasing 1.0.0{}: from then on a breaking change is a major release, and peers and \
         consumers rely on that",
        last_release
            .map(|v| format!(" (from {v})"))
            .unwrap_or_default()
    ));

    let mut protocols = Vec::new();
    let mut given_up = Vec::new();
    let mut deprecated = Vec::new();
    for schema in &context.schema_contexts {
        let schema = schema.borrow();
        let frozen = schema.frozen_schema.borrow();
        let Some(units) = frozen.as_ref() else {
            continue;
        };
        for unit in units {
            let (name, parameters) = match unit {
                FrozenUnit::Protocol {
                    name, parameters, ..
                } => {
                    protocols.push(format!(
                        "{name} (handshake `{}`)",
                        unverified::handshake(parameters)
                    ));
                    let set = unverified::frozen_set(parameters);
                    if !set.is_empty() {
                        given_up.push(format!("{name} gives up {}", unverified::describe(set)));
                    }
                    (name, parameters)
                }
                FrozenUnit::Struct {
                    name, parameters, ..
                }
                | FrozenUnit::Error {
                    name, parameters, ..
                }
                | FrozenUnit::TypeAlias {
                    name, parameters, ..
                } => (name, parameters),
                _ => continue,
            };
            if parameters.iter().any(|p| matches!(p, FrozenUnit::Deprecated(d) if !matches!(d, comline_core::schema::ir::frozen::unit::DeprecatedValue::False))) {
                deprecated.push(name.clone());
            }
        }
    }
    if protocols.is_empty() {
        ui::detail("protocols: none".to_string());
    } else {
        ui::detail(format!("protocols: {}", protocols.join(", ")));
    }
    if !given_up.is_empty() || !deprecated.is_empty() {
        ui::note("worth settling first (not required):");
        for line in given_up {
            ui::detail(line);
        }
        if !deprecated.is_empty() {
            ui::detail(format!(
                "deprecated, and kept until 2.0: {}",
                deprecated.join(", ")
            ));
        }
    }
}

/// Ask on a terminal (default no); without one, `yes` is the only way on.
fn confirm(question: &str, yes: bool, what: &'static str) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if !io::stdin().is_terminal() {
        return Err(CliError::ReleaseConfirmationRequired { what }.into());
    }
    print!("{question}? [y/N] ");
    io::stdout().flush().into_diagnostic()?;
    let mut line = String::new();
    io::stdin().read_line(&mut line).into_diagnostic()?;
    Ok(matches!(
        line.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}
