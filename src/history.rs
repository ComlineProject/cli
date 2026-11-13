//! Reading the CAS commit chain — shared by `diff` and `generation`.
//!
//! Comline's CAS is an append-only linear chain (no branches), so we walk it
//! once from `refs/heads/main` and resolve specs against that list.

use std::path::Path;

use crate::error::CliError;

use comline_core::package::build::cas::object_store::{ensure_compatible_store, StoreState};
use comline_core::package::build::cas::objects::{Commit, EntryMode, Tree};
use comline_core::package::build::cas::storage::Hash;
use comline_core::package::build::cas::{refs, ObjectStore};
use comline_core::package::config::ir::frozen::cas::blob::{read_config, StoredConfigs};
use comline_core::package::config::ir::frozen::FrozenUnit as ConfigUnit;
use comline_core::schema::ir::frozen::cas::blob::load_schema_from_tree;
use comline_core::schema::ir::frozen::unit::FrozenUnit;
use miette::Result;

/// One commit in the chain, with its object hash.
pub struct Entry {
    pub hash: Hash,
    pub commit: Commit,
}

/// Refuse to use a `.comline/` store this comline can't read or write.
///
/// The stamp (`.comline/FORMAT_VERSION`) says which IR format the store's
/// objects are in. Decoding another format's objects gives garbage or a
/// confusing decode error, so each read goes through this first, and a
/// store of another format is refused by every command, `build` included:
/// nothing here deletes, moves or rewrites it (core's
/// [`ensure_compatible_store`] is read-only). [`load`] and [`open_store`]
/// call it; a command that reads the store some other way calls it directly.
///
/// No `.comline/`, or one with no stamp and no history, is "nothing built
/// yet": fine here, and the caller reports it as it always did.
pub fn ensure_readable(work_dir: &Path) -> Result<()> {
    match ensure_compatible_store(work_dir) {
        Ok(StoreState::Fresh | StoreState::Current) => Ok(()),
        Err(e) => Err(CliError::Store(e).into()),
    }
}

/// The object store of `work_dir`, after [`ensure_readable`].
pub fn open_store(work_dir: &Path) -> Result<ObjectStore> {
    ensure_readable(work_dir)?;
    Ok(ObjectStore::new(work_dir))
}

/// The whole commit chain for `work_dir`, newest-first.
///
/// Errors with [`CliError::NothingBuilt`] if the project has never been built,
/// and with [`CliError::Store`] if its store can't be used.
pub fn load(work_dir: &Path) -> Result<Vec<Entry>> {
    ensure_readable(work_dir)?;
    if !refs::ref_exists(work_dir, refs::main_ref()) {
        return Err(CliError::NothingBuilt(work_dir.to_path_buf()).into());
    }
    let store = open_store(work_dir)?;
    let head = refs::read_ref(work_dir, refs::main_ref())
        .map_err(|e| miette::miette!("failed to read HEAD ref: {e:#}"))?;
    walk(&store, head)
}

/// Collect commits newest-first by following the first parent from `head`.
fn walk(store: &ObjectStore, head: Hash) -> Result<Vec<Entry>> {
    let mut entries = Vec::new();
    let mut next = Some(head);
    while let Some(hash) = next {
        let bytes = store
            .read(&hash)
            .map_err(|e| miette::miette!("failed to read commit {hash}: {e:#}"))?;
        let commit = Commit::from_bytes(&bytes)
            .map_err(|e| miette::miette!("corrupt commit {hash}: {e:#}"))?;
        next = commit.parents.first().copied();
        entries.push(Entry { hash, commit });
    }
    Ok(entries)
}

/// Resolve `HEAD`, a version string, or a full/short (>= 4 char) commit hash to a
/// commit in `history`.
pub fn resolve<'a>(history: &'a [Entry], spec: &str) -> Result<&'a Commit> {
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

/// The package config `commit` was built with; empty for a commit made
/// before config was stored.
pub fn load_config(store: &ObjectStore, commit: &Commit) -> Result<Vec<ConfigUnit>> {
    let root_bytes = store
        .read(&commit.tree)
        .map_err(|e| miette::miette!("failed to read root tree: {e:#}"))?;
    let root =
        Tree::from_bytes(&root_bytes).map_err(|e| miette::miette!("corrupt root tree: {e:#}"))?;
    root.entries
        .iter()
        .find(|e| e.name == "config" && e.mode == EntryMode::Blob)
        .map(|e| read_config(store, &e.hash))
        .transpose()
        .map(Option::unwrap_or_default)
        .map_err(|e| miette::miette!("failed to read the package config: {e:#}"))
}

/// Every config `commit` holds: the package's own and each vendored
/// dependency's, for the encodings its changelogs are judged under.
pub fn load_stored_configs(store: &ObjectStore, commit: &Commit) -> Result<StoredConfigs> {
    let root_bytes = store
        .read(&commit.tree)
        .map_err(|e| miette::miette!("failed to read root tree: {e:#}"))?;
    let root =
        Tree::from_bytes(&root_bytes).map_err(|e| miette::miette!("corrupt root tree: {e:#}"))?;
    StoredConfigs::read(store, &root)
        .map_err(|e| miette::miette!("failed to read the package's configs: {e:#}"))
}

/// Load every local schema's frozen units from a commit's root tree, in tree
/// order. Vendored dependencies (`dep_<name>` trees) aren't this package's
/// schemas and are skipped.
pub fn load_schemas(store: &ObjectStore, commit: &Commit) -> Result<Vec<Vec<FrozenUnit>>> {
    let root_bytes = store
        .read(&commit.tree)
        .map_err(|e| miette::miette!("failed to read root tree: {e:#}"))?;
    let root =
        Tree::from_bytes(&root_bytes).map_err(|e| miette::miette!("corrupt root tree: {e:#}"))?;

    let mut schemas = Vec::new();
    for entry in &root.entries {
        if entry.mode != EntryMode::Tree || !entry.name.starts_with("schema_") {
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
