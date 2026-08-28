//! `comline diff <old> <new>` — schema changelog between two frozen versions.
//!
//! Comline's CAS is an append-only linear commit chain (no branches), so we walk
//! it once from `refs/heads/main` and resolve each argument against that list.

use std::path::Path;

use comline_core::package::build::cas::objects::{Commit, EntryMode, Tree};
use comline_core::package::build::cas::storage::Hash;
use comline_core::package::build::cas::{refs, ObjectStore};
use comline_core::schema::ir::diff::{analyze_schema_changes, SchemaChanges};
use comline_core::schema::ir::frozen::cas::blob::load_schema_from_tree;
use comline_core::schema::ir::frozen::unit::FrozenUnit;
use miette::Result;

use crate::changes;
use crate::commands::ensure_project;
use crate::error::CliError;
use crate::ui;

pub fn run(work_dir: &Path, old: &str, new: &str) -> Result<()> {
    ensure_project(work_dir)?;

    if !refs::ref_exists(work_dir, refs::main_ref()) {
        return Err(CliError::NothingBuilt(work_dir.to_path_buf()).into());
    }

    let store = ObjectStore::new(work_dir);
    let head = refs::read_ref(work_dir, refs::main_ref())
        .map_err(|e| miette::miette!("failed to read HEAD ref: {e:#}"))?;
    let history = walk_history(&store, head)?;

    let old_commit = resolve(&history, old)?;
    let new_commit = resolve(&history, new)?;

    ui::step(format!(
        "Diff {} → {}",
        label(old, old_commit),
        label(new, new_commit)
    ));

    if old_commit.tree == new_commit.tree {
        ui::detail("identical — no schema changes");
        return Ok(());
    }

    let old_schemas = load_schemas(&store, old_commit)?;
    let new_schemas = load_schemas(&store, new_commit)?;
    changes::render(&aggregate(&old_schemas, &new_schemas));
    Ok(())
}

struct HistoryEntry {
    hash: Hash,
    commit: Commit,
}

/// Collect commits newest-first by following the first parent from `head`.
fn walk_history(store: &ObjectStore, head: Hash) -> Result<Vec<HistoryEntry>> {
    let mut entries = Vec::new();
    let mut next = Some(head);
    while let Some(hash) = next {
        let bytes = store
            .read(&hash)
            .map_err(|e| miette::miette!("failed to read commit {hash}: {e:#}"))?;
        let commit = Commit::from_bytes(&bytes)
            .map_err(|e| miette::miette!("corrupt commit {hash}: {e:#}"))?;
        next = commit.parents.first().copied();
        entries.push(HistoryEntry { hash, commit });
    }
    Ok(entries)
}

/// Resolve `HEAD`, a version string, or a full/short commit hash to a commit.
fn resolve<'a>(history: &'a [HistoryEntry], spec: &str) -> Result<&'a Commit> {
    if spec.eq_ignore_ascii_case("HEAD") {
        return history
            .first()
            .map(|e| &e.commit)
            .ok_or_else(|| miette::miette!("history is empty"));
    }
    if let Some(entry) = history.iter().find(|e| e.commit.version == spec) {
        return Ok(&entry.commit);
    }
    if let Some(entry) = history.iter().find(|e| e.hash.to_hex() == spec) {
        return Ok(&entry.commit);
    }
    if spec.len() >= 4 {
        let mut matches = history.iter().filter(|e| e.hash.to_hex().starts_with(spec));
        if let (Some(entry), None) = (matches.next(), matches.next()) {
            return Ok(&entry.commit);
        }
    }

    let available: Vec<&str> = history.iter().map(|e| e.commit.version.as_str()).collect();
    Err(miette::miette!(
        "no built version matches `{spec}` (available: {})",
        available.join(", ")
    ))
}

/// Load every schema's frozen units from a commit's root tree, in tree order.
fn load_schemas(store: &ObjectStore, commit: &Commit) -> Result<Vec<Vec<FrozenUnit>>> {
    let root_bytes = store
        .read(&commit.tree)
        .map_err(|e| miette::miette!("failed to read root tree: {e:#}"))?;
    let root =
        Tree::from_bytes(&root_bytes).map_err(|e| miette::miette!("corrupt root tree: {e:#}"))?;

    let mut schemas = Vec::new();
    for entry in &root.entries {
        if entry.mode != EntryMode::Tree {
            continue;
        }
        let sub_bytes = store
            .read(&entry.hash)
            .map_err(|e| miette::miette!("failed to read schema tree `{}`: {e:#}", entry.name))?;
        let sub = Tree::from_bytes(&sub_bytes)
            .map_err(|e| miette::miette!("corrupt schema tree `{}`: {e:#}", entry.name))?;
        let units = load_schema_from_tree(store, &sub)
            .map_err(|e| miette::miette!("failed to load schema `{}`: {e:#}", entry.name))?;
        schemas.push(units);
    }
    Ok(schemas)
}

/// Diff schema-by-schema (paired by position) and merge the results, treating
/// added/removed schema files as wholesale additions/removals.
fn aggregate(old: &[Vec<FrozenUnit>], new: &[Vec<FrozenUnit>]) -> SchemaChanges {
    let mut merged = SchemaChanges::default();
    let common = old.len().min(new.len());

    for i in 0..common {
        extend(&mut merged, analyze_schema_changes(&old[i], &new[i]));
    }
    for schema in &new[common..] {
        extend(&mut merged, analyze_schema_changes(&[], schema));
    }
    for schema in &old[common..] {
        extend(&mut merged, analyze_schema_changes(schema, &[]));
    }
    merged
}

fn extend(into: &mut SchemaChanges, from: SchemaChanges) {
    into.breaking_changes.extend(from.breaking_changes);
    into.new_features.extend(from.new_features);
    into.modifications.extend(from.modifications);
}

fn label(spec: &str, commit: &Commit) -> String {
    if spec.eq_ignore_ascii_case("HEAD") {
        format!("HEAD ({})", commit.version)
    } else if spec == commit.version {
        commit.version.clone()
    } else {
        format!("{spec} ({})", commit.version)
    }
}
