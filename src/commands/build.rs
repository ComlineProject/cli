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

    ui::step(format!("Building project{}", ui::at_path(work_dir)));
    let spinner = ui::spinner("compiling schemas");
    let result = build::build(work_dir);
    ui::finish_spinner(spinner);

    let result = result.map_err(|e| core_err("build failed", e))?;
    for warning in result.context.rendered_warnings() {
        ui::warn(warning);
    }

    ui::success("Project built");

    if result.is_initial_build() {
        ui::detail(pkg(format!("initial build {}", result.current_version)));
        next_release_line(&result);
        return Ok(());
    }

    let has_changes = result
        .schema_changes
        .as_ref()
        .map(|c| !c.is_empty())
        .unwrap_or(false)
        || !result.dependency_changes.is_empty();

    if !has_changes {
        ui::detail(pkg(format!(
            "version {} (no changes)",
            result.current_version
        )));
        next_release_line(&result);
        return Ok(());
    }

    if let Some(version_change) = result.version_change() {
        ui::detail(pkg(version_change));
    }
    if let Some(changes) = &result.schema_changes {
        changes::render(changes);
    }
    for dependency in &result.dependency_changes {
        ui::group(format!("In dependency `{}`:", dependency.name));
        changes::render(&dependency.changes);
    }
    ui::detail(bump_line(result.version_bump));
    next_release_line(&result);

    Ok(())
}

/// What `comline release` would make now: the release this build leads to.
/// Nothing when the latest commit is itself a release.
fn next_release_line(result: &build::BuildResult) {
    // A build's version is the release it leads to plus `-dev.N`.
    let Some((next, _)) = result.current_version.split_once('-') else {
        return;
    };
    let since = match &result.last_release {
        Some(last) => format!(" ({} since {last})", bump_word(result.release_bump)),
        None => " (the first release)".to_string(),
    };
    ui::detail(format!(
        "next release: {next}{since} — run `comline release`"
    ));
}

fn bump_word(bump: VersionBump) -> &'static str {
    match bump {
        VersionBump::Major => "breaking changes",
        VersionBump::Minor => "new features",
        VersionBump::Patch => "modifications",
        VersionBump::None => "nothing that calls for a bump",
    }
}

/// A `📦`-prefixed package line, or plain text (with an ASCII arrow) under `--plain`.
fn pkg(body: String) -> String {
    if ui::plain() {
        body.replace(" → ", " -> ")
    } else {
        format!("📦 {body}")
    }
}

fn bump_line(bump: VersionBump) -> String {
    let text = match bump {
        VersionBump::Major => "major version bump (breaking changes)",
        VersionBump::Minor => "minor version bump (new features)",
        VersionBump::Patch => "patch version bump (modifications)",
        VersionBump::None => "no version bump",
    };
    if ui::plain() {
        text.to_string()
    } else {
        format!("⬆️  {text}")
    }
}
