//! `comline build` — compile, validate and freeze a new immutable version.

use std::path::Path;

use comline_core::package::build::{self, VersionBump};
use miette::Result;

use crate::changes;
use crate::commands::{core_err, ensure_project};
use crate::ui;
use crate::watch;

pub fn run(work_dir: &Path, release: bool, watching: bool) -> Result<()> {
    ensure_project(work_dir)?;

    if watching {
        watch::run(work_dir, "build", || build_once(work_dir, release))
    } else {
        build_once(work_dir, release)
    }
}

fn build_once(work_dir: &Path, release: bool) -> Result<()> {
    if release {
        ui::note("--release is reserved and currently has no effect");
    }

    ui::step("Building project");
    let spinner = ui::spinner("compiling schemas");
    let result = build::build(work_dir);
    ui::finish_spinner(spinner);

    let result = result.map_err(|e| core_err("build failed", e))?;

    ui::success("Project built");

    if result.is_initial_build() {
        ui::detail(format!("📦 initial version {}", result.current_version));
        return Ok(());
    }

    let has_changes = result
        .schema_changes
        .as_ref()
        .map(|c| !c.is_empty())
        .unwrap_or(false);

    if !has_changes {
        ui::detail(format!(
            "📦 version {} (no changes)",
            result.current_version
        ));
        return Ok(());
    }

    if let Some(version_change) = result.version_change() {
        ui::detail(format!("📦 version {version_change}"));
    }
    if let Some(changes) = &result.schema_changes {
        changes::render(changes);
    }
    ui::detail(bump_line(result.version_bump));

    Ok(())
}

fn bump_line(bump: VersionBump) -> &'static str {
    match bump {
        VersionBump::Major => "⬆️  major version bump (breaking changes)",
        VersionBump::Minor => "⬆️  minor version bump (new features)",
        VersionBump::Patch => "⬆️  patch version bump (modifications)",
        VersionBump::None => "no version bump",
    }
}
