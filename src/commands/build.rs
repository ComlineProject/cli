//! `comline build` — compile, validate and freeze a new immutable version.
//!
//! Before freezing, every target declared under `code_generation.languages`
//! is proven against the generators this binary was built with
//! ([`lang_targets::check_declared`]): a declaration is a promise consumers
//! rely on.

use std::path::Path;

use comline_core::package::build::cas::object_store::StoreError;
use comline_core::package::build::cas::objects::Commit;
use comline_core::package::build::cas::reconcile::{self, Heads};
use comline_core::package::build::cas::{main_ref, read_ref, ref_exists, Hash, ObjectStore};
use comline_core::package::build::{self, VersionBump};
use miette::Result;

use crate::bounds_migration;
use crate::changes;
use crate::commands::generation::generator_registry;
use crate::commands::{core_err, ensure_project};
use crate::error;
use crate::lang_targets;
use crate::ui;
use crate::watch;

pub fn run(work_dir: &Path, watching: bool, reconcile: Option<&str>) -> Result<()> {
    ensure_project(work_dir)?;

    if let Some(other) = reconcile {
        mark_diverged(work_dir, other)?;
    }
    if watching {
        watch::run(work_dir, "build", || build_once(work_dir, false))
    } else {
        build_once(work_dir, true)
    }
}

/// `--reconcile <hash>`: put the dropped head back beside the current one,
/// as a Git conflict leaves them, for the build to reconcile (cli#128).
fn mark_diverged(work_dir: &Path, other: &str) -> Result<()> {
    let hash = Hash::from_hex(other.trim())
        .map_err(|e| miette::miette!("`--reconcile {other}`: not a commit hash: {e}"))?;
    reconcile::mark_diverged(work_dir, &hash)
        .map_err(|e| core_err(&format!("couldn't reconcile with {other}"), e))?;
    ui::note(format!(
        "marked {} as a second head to reconcile",
        short(&hash)
    ));
    Ok(())
}

/// The two heads a Git merge left in `.comline/refs/heads/main`, if it holds
/// two: this build reconciles them.
fn diverged_heads(work_dir: &Path) -> Option<(Hash, Hash)> {
    if !ref_exists(work_dir, main_ref()) {
        return None;
    }
    match reconcile::read_heads(work_dir) {
        Ok(Heads::Two(ours, theirs)) => Some((ours, theirs)),
        _ => None,
    }
}

/// Say how a build joined two heads: which one the line continued from,
/// and which one it recorded as the second parent.
fn report_reconciled(work_dir: &Path, (ours, theirs): (Hash, Hash)) {
    let parents = read_ref(work_dir, main_ref()).ok().and_then(|head| {
        let bytes = ObjectStore::new(work_dir).read(&head).ok()?;
        Some((head, Commit::from_bytes(&bytes).ok()?.parents))
    });
    let message = match parents {
        Some((_, parents)) if parents.len() == 2 => format!(
            "reconciled the diverged history: continued from {}, recorded {} as the second parent",
            short(&parents[0]),
            short(&parents[1])
        ),
        Some((head, _)) => format!(
            "reconciled the diverged history ({} and {}): continued from {}, nothing new to record",
            short(&ours),
            short(&theirs),
            short(&head)
        ),
        None => format!(
            "reconciled the diverged history ({} and {})",
            short(&ours),
            short(&theirs)
        ),
    };
    ui::detail(message);
}

fn short(hash: &Hash) -> String {
    hash.to_hex()[..12].to_string()
}

fn build_once(work_dir: &Path, offer_migration: bool) -> Result<()> {
    ui::step(format!("Building project{}", ui::at_path(work_dir)));
    // Every package has an id (its identity for `comline@2`); one without
    // gets it here, as one line in config.idp.
    match comline_core::package::id::ensure_in_config(work_dir) {
        Ok(Some(id)) => ui::note(format!(
            "config.idp had no `id`: added `id = \"{id}\"`. Keep it when renaming the \
             package, and change it in a copy"
        )),
        Ok(None) => {}
        Err(e) => return Err(core_err("couldn't add an id to config.idp", e)),
    }
    let diverged = diverged_heads(work_dir);
    let spinner = ui::spinner("compiling schemas");
    let registry = generator_registry();
    let mut target_warnings = Vec::new();
    let result = build::build_with(work_dir, |context| {
        target_warnings = lang_targets::check_declared(context, &registry)?;
        Ok(())
    });
    ui::finish_spinner(spinner);

    let result = match result {
        Ok(result) => result,
        // A store core refuses (another IR format, no stamp) is a
        // precondition, reported as core words it, with nothing written.
        Err(e) if e.downcast_ref::<StoreError>().is_some() => {
            return Err(error::from_core("build failed", e));
        }
        Err(e) => {
            let detail = format!("{e:#}");
            let unbounded = bounds_migration::Unbounded::of(&detail);
            if unbounded.is_empty() {
                return Err(core_err("build failed", e));
            }
            if !(offer_migration && bounds_migration::interactive()) {
                let hint = bounds_migration::hint(&unbounded);
                return Err(miette::miette!("build failed:\n\n{detail}\n{hint}"));
            }
            // Errors first, then the question.
            ui::error(format!("build failed:\n\n{detail}"));
            ui::note(bounds_migration::hint(&unbounded));
            if bounds_migration::offer(work_dir, &unbounded) {
                return build_once(work_dir, false);
            }
            return Err(miette::miette!(
                "build failed: {} values without a maximum",
                unbounded.total()
            ));
        }
    };
    for warning in result
        .context
        .rendered_warnings()
        .into_iter()
        .chain(target_warnings)
    {
        ui::warn(warning);
    }

    ui::success("Project built");
    if let Some(heads) = diverged {
        report_reconciled(work_dir, heads);
    }

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
        || !result.dependency_changes.is_empty()
        || !result.config_changes.is_empty()
        || !result.unseen.is_empty();

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
    changes::render_config(&result.config_changes);
    changes::render_unseen(&result.unseen);
    ui::detail(changes::bump_line(result.version_bump));
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
