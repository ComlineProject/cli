//! `comline doc` — the package's documentation (core#145, generation#101).
//!
//! Core builds the documentation model (`comline_core::doc`): from a built
//! version's frozen IR and `//!` headers, or from the compiled working tree.
//! By default every version is documented: each release, each dev build and
//! the unreleased working state (when it differs from the last build), each
//! with the changelog from the version before it (a step), computed by
//! core's rules. A release also carries its changes since the previous
//! release, compared directly between the two built trees, which its
//! changelog page shows; a dev build (and the unreleased state) its changes
//! since the last release, which its page links to. The compare page
//! composes the steps only.
//!
//! Only the package's own modules are documented as its modules: what it
//! reaches of each dependency (and `std`) is listed apart, by the reach
//! versioning uses (`comline_core::doc`).
//! `[doc] versions` trims the list; `--version` documents one version alone
//! (with `--changelog-from` adding a changelog from an earlier one).
//!
//! Each of `[doc] formats` lands in `<out>/<format>/`: one folder per version
//! (`0.2.0/`, `0.3.0-dev.2/`, `unreleased/`), then the site's index, which
//! the renderer writes at the format's root when it renders indexes
//! (`RendererInfo::sites`): for HTML `index.html` pointing at the latest, the
//! picker's `versions.js` and the comparison page; for Markdown an
//! `index.md`. `json` writes each version's model (`model.json`) and an
//! `index.json` of the versions. `markdown` and `html` are the official
//! renderers, linked in like the official generators; `[doc] renderer` names
//! an external one instead, run exactly like an external generator
//! ([`crate::external`]: the same sandbox, limits and path checks).
//!
//! **Caching.** A built version never changes, so its folder is kept between
//! runs: `<out>/<format>/.comline-doc.json` records, per version folder, the
//! key it was rendered with (its commit, the commit of the version before it,
//! the renderer and its version, the model version and the links). A folder
//! whose key matches is left alone; the unreleased state always re-renders.
//! A folder for a version no longer listed is removed.
//!
//! Like `check`, nothing is written to `.comline/`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use comline_codegen::config::{
    ComlineToml, Doc, DocVersions, DEFAULT_DOC_FORMATS, DEFAULT_DOC_OUT,
};
use comline_codegen::protocol::{
    self, Description, FileOut, RenderOptions, RenderRequest, SiteVersion, VersionKind,
};
use comline_codegen::Registry;
use comline_core::doc::{self, ChangelogSpan, DocModel, FrozenDocs, DOC_MODEL_VERSION};
use comline_core::package::build::cas::objects::{Commit, Tree};
use comline_core::package::build::cas::storage::Hash;
use comline_core::package::build::cas::{refs, ObjectStore};
use comline_core::schema::ir::frozen::unit::FrozenUnit;
use miette::{miette, IntoDiagnostic, Result, WrapErr};
use serde::{Deserialize, Serialize};

use crate::commands::{core_err, ensure_project};
use crate::{external, history, ui};

/// The command line's say, each overriding its `[doc]` key.
#[derive(Debug, Default)]
pub struct Flags {
    pub out: Option<PathBuf>,
    pub version: Option<String>,
    pub changelog_from: Option<String>,
    pub formats: Vec<String>,
}

/// The cache manifest's file name, in each format's folder.
pub const MANIFEST: &str = ".comline-doc.json";

/// The unreleased state's folder.
pub const UNRELEASED: &str = "unreleased";

/// The renderers linked into this binary.
pub(crate) fn renderer_registry() -> Registry {
    let mut registry = Registry::new();
    comline_doc_markdown::register(&mut registry);
    comline_doc_html::register(&mut registry);
    registry
}

/// One version to document.
struct Version {
    /// Its folder: the full version, or [`UNRELEASED`].
    id: String,
    release: String,
    kind: VersionKind,
    /// Its commit; `None` for the unreleased state.
    commit: Option<Hash>,
    model: DocModel,
    /// Its own frozen schemas: not its dependencies' or `std`'s.
    schemas: Vec<Vec<FrozenUnit>>,
    /// The commits its changelogs are computed from, for the cache key.
    changelog_from: Vec<Hash>,
    /// What its documentation lacks.
    notes: Vec<String>,
}

pub fn run(work_dir: &Path, flags: Flags) -> Result<()> {
    ensure_project(work_dir)?;
    let cfg = ComlineToml::load(work_dir)?.doc;

    let out = match (&flags.out, &cfg.out) {
        (Some(out), _) => work_dir.join(out),
        (None, Some(out)) => work_dir.join(out),
        (None, None) => work_dir.join(DEFAULT_DOC_OUT),
    };
    let mut formats: Vec<String> = if !flags.formats.is_empty() {
        flags.formats.clone()
    } else {
        cfg.formats
            .clone()
            .unwrap_or_else(|| DEFAULT_DOC_FORMATS.iter().map(|f| f.to_string()).collect())
    };
    let mut seen = std::collections::BTreeSet::new();
    formats.retain(|f| seen.insert(f.clone()));
    if formats.is_empty() {
        return Err(miette!(
            "`[doc] formats` is empty: name at least one format"
        ));
    }
    let changelog_from = flags.changelog_from.or(cfg.changelog_from.clone());
    let renderers = Renderers::new(work_dir, &cfg)?;

    if let Some(spec) = &flags.version {
        let mut version = frozen_version(work_dir, spec)?;
        if let Some(from) = &changelog_from {
            let history = history::load(work_dir)?;
            let commit = history::resolve(&history, from)?;
            let old = history::load_schemas(&ObjectStore::new(work_dir), commit)?;
            version.model.changelog.push(doc::changelog(
                &commit.version,
                &old,
                &version.id,
                &version.schemas,
            ));
        }
        return document_one(&out, &formats, &renderers, &cfg, version);
    }
    if changelog_from.is_some() {
        ui::warn(
            "`changelog_from` only applies with `--version`: every version already carries \
             the changelog from the one before it",
        );
    }
    let versions = every_version(work_dir, cfg.versions.as_ref())?;
    if versions.is_empty() {
        return Err(miette!("`[doc] versions` leaves no version to document"));
    }

    for format in &formats {
        let dir = out.join(format);
        let (rendered, kept) = document_site(&dir, format, &renderers, &cfg, &versions)?;
        ui::success(format!(
            "Documented {} version(s) of {} into {} ({rendered} rendered, {kept} unchanged)",
            versions.len(),
            versions[0].model.package.name,
            dir.display()
        ));
    }
    Ok(())
}

/// `--version`: one version, into `<out>/<format>/<version>/`. The cache
/// forgets the folder, since what's in it may differ from a full run's.
fn document_one(
    out: &Path,
    formats: &[String],
    renderers: &Renderers,
    cfg: &Doc,
    version: Version,
) -> Result<()> {
    for format in formats {
        let dir = out.join(format);
        let files = renderers.render_version(format, cfg, &version)?;
        let target = dir.join(&version.id);
        replace_dir(&target)?;
        write_files(&target, &files)?;
        let mut manifest = Manifest::read(&dir);
        if manifest.versions.remove(&version.id).is_some() {
            manifest.write(&dir)?;
        }
        ui::success(format!(
            "Documented {} {} into {} ({} file(s))",
            version.model.package.name,
            version.id,
            target.display(),
            files.len()
        ));
    }
    Ok(())
}

/// Every version, each rendered unless its cached folder still matches,
/// then the site's index. Returns how many were rendered and kept.
fn document_site(
    dir: &Path,
    format: &str,
    renderers: &Renderers,
    cfg: &Doc,
    versions: &[Version],
) -> Result<(usize, usize)> {
    let old = Manifest::read(dir);
    let mut manifest = Manifest::default();
    let renderer_key = renderers.key(format);
    let (mut rendered, mut kept) = (0, 0);
    for version in versions {
        let key = version
            .commit
            .map(|commit| cache_key(&commit, &version.changelog_from, &renderer_key, &cfg.links));
        let target = dir.join(&version.id);
        if old.reuses(&version.id, &key) && target.is_dir() {
            manifest.versions.insert(version.id.clone(), key);
            kept += 1;
            continue;
        }
        let files = renderers.render_version(format, cfg, version)?;
        replace_dir(&target)?;
        write_files(&target, &files)?;
        manifest.versions.insert(version.id.clone(), key);
        rendered += 1;
    }
    // A version no longer listed (trimmed away, or the unreleased state now
    // built): its folder goes.
    for id in old.versions.keys() {
        if !manifest.versions.contains_key(id) {
            let stale = dir.join(id);
            if stale.is_dir() {
                std::fs::remove_dir_all(&stale)
                    .into_diagnostic()
                    .wrap_err_with(|| format!("failed to remove `{}`", stale.display()))?;
            }
        }
    }
    let index = renderers.render_index(format, cfg, versions)?;
    write_files(dir, &index)?;
    manifest.write(dir)?;
    Ok((rendered, kept))
}

/// `<out>/<format>/.comline-doc.json`: each version folder's cache key
/// (`null` for the unreleased state, never reused).
#[derive(Debug, Default, Serialize, Deserialize)]
struct Manifest {
    versions: BTreeMap<String, Option<String>>,
}

/// A built version folder's cache key: everything its pages are rendered
/// from (its commit and those its changelogs start from). `renderer_key` is
/// [`Renderers::key`], which moves whenever the renderer's output can (a
/// built-in's version carries a content hash of its sources and assets).
fn cache_key(
    commit: &Hash,
    changelog_from: &[Hash],
    renderer_key: &str,
    links: &BTreeMap<String, String>,
) -> String {
    let previous = if changelog_from.is_empty() {
        "none".to_string()
    } else {
        changelog_from.iter().map(|c| c.to_hex()).collect::<Vec<_>>().join(" and ")
    };
    format!(
        "commit {} after {previous}; {renderer_key}; model {DOC_MODEL_VERSION}; links {links:?}",
        commit.to_hex()
    )
}

impl Manifest {
    /// Whether the folder `id` was rendered with `key`; never for the
    /// unreleased state, which has none.
    fn reuses(&self, id: &str, key: &Option<String>) -> bool {
        key.is_some() && self.versions.get(id) == Some(key)
    }

    /// The manifest in `dir`; empty when there's none or it doesn't parse.
    fn read(dir: &Path) -> Self {
        std::fs::read_to_string(dir.join(MANIFEST))
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    fn write(&self, dir: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self).into_diagnostic()?;
        write_files(
            dir,
            &[FileOut {
                path: MANIFEST.into(),
                contents: format!("{json}\n").into_bytes(),
            }],
        )
    }
}

/// The renderer side: the built-ins, or `[doc] renderer`.
struct Renderers {
    registry: Registry,
    external: Option<External>,
}

struct External {
    renderer: external::External,
    description: Description,
    /// The renderer file's hash, part of the cache key.
    hash: String,
}

impl Renderers {
    fn new(work_dir: &Path, cfg: &Doc) -> Result<Self> {
        let external = match &cfg.renderer {
            Some(path) => {
                let path = work_dir.join(path);
                let timeout = cfg
                    .timeout
                    .map(Duration::from_secs)
                    .unwrap_or(external::DEFAULT_TIMEOUT);
                let renderer = external::External::open(&path, timeout)?;
                let description = renderer.describe()?;
                let bytes = std::fs::read(&path)
                    .into_diagnostic()
                    .wrap_err_with(|| format!("failed to read `{}`", path.display()))?;
                Some(External {
                    renderer,
                    description,
                    hash: Hash::from_bytes(&bytes).to_hex(),
                })
            }
            None => None,
        };
        Ok(Renderers {
            registry: renderer_registry(),
            external,
        })
    }

    fn description(&self) -> Description {
        match &self.external {
            Some(external) => external.description.clone(),
            None => protocol::describe(&self.registry),
        }
    }

    /// What identifies `format`'s renderer in the cache key.
    fn key(&self, format: &str) -> String {
        if format == "json" {
            return format!("json via comline {}", env!("CARGO_PKG_VERSION"));
        }
        let version = self
            .description()
            .renderers
            .into_iter()
            .find(|r| r.id == format)
            .map(|r| r.version)
            .unwrap_or_default();
        match &self.external {
            Some(external) => format!("{format}@{version} from {}", external.hash),
            None => format!(
                "{format}@{version} via comline {}",
                env!("CARGO_PKG_VERSION")
            ),
        }
    }

    fn render(&self, request: RenderRequest) -> Result<Vec<FileOut>> {
        match &self.external {
            Some(external) => external.renderer.render(&external.description, request),
            None => external::builtin_render(&self.registry, request),
        }
    }

    /// One version's files, relative to its folder.
    fn render_version(&self, format: &str, cfg: &Doc, version: &Version) -> Result<Vec<FileOut>> {
        if format == "json" {
            let json = serde_json::to_string_pretty(&version.model).into_diagnostic()?;
            return Ok(vec![FileOut {
                path: "model.json".into(),
                contents: format!("{json}\n").into_bytes(),
            }]);
        }
        self.render(RenderRequest {
            renderer: format.to_string(),
            model_version: DOC_MODEL_VERSION,
            model: serde_json::to_string(&version.model).into_diagnostic()?,
            options: RenderOptions {
                links: cfg.links.clone(),
                version: Some(version.id.clone()),
                versions: vec![],
                notes: version.notes.clone(),
            },
        })
    }

    /// The site's index, relative to the format's folder: `index.json` for
    /// `json`, the renderer's own when it renders indexes, else nothing.
    fn render_index(&self, format: &str, cfg: &Doc, versions: &[Version]) -> Result<Vec<FileOut>> {
        let latest = latest(versions);
        if format == "json" {
            let list: Vec<serde_json::Value> = versions
                .iter()
                .map(|v| {
                    serde_json::json!({
                        "id": v.id, "release": v.release, "kind": v.kind.as_str(),
                        "model": format!("{}/model.json", v.id),
                    })
                })
                .collect();
            let index = serde_json::json!({
                "package": latest.model.package.name,
                "model_version": DOC_MODEL_VERSION,
                "latest": latest.id,
                "versions": list,
            });
            let json = serde_json::to_string_pretty(&index).into_diagnostic()?;
            return Ok(vec![FileOut {
                path: "index.json".into(),
                contents: format!("{json}\n").into_bytes(),
            }]);
        }
        let sites = self
            .description()
            .renderers
            .iter()
            .any(|r| r.id == format && r.sites);
        if !sites {
            ui::note(format!(
                "the `{format}` renderer doesn't render a site index: each version has its own folder"
            ));
            return Ok(vec![]);
        }
        let mut site = vec![];
        for v in versions {
            site.push(SiteVersion {
                id: v.id.clone(),
                release: v.release.clone(),
                kind: v.kind,
                model: serde_json::to_string(&v.model).into_diagnostic()?,
            });
        }
        self.render(RenderRequest {
            renderer: format.to_string(),
            model_version: DOC_MODEL_VERSION,
            model: serde_json::to_string(&latest.model).into_diagnostic()?,
            options: RenderOptions {
                links: cfg.links.clone(),
                version: None,
                versions: site,
                notes: vec![],
            },
        })
    }
}

/// The newest release, else the newest dev build, else the unreleased state.
fn latest(versions: &[Version]) -> &Version {
    [
        VersionKind::Release,
        VersionKind::Dev,
        VersionKind::Unreleased,
    ]
    .into_iter()
    .find_map(|kind| versions.iter().rev().find(|v| v.kind == kind))
    .expect("at least one version")
}

/// Every built version, oldest first, then the unreleased state when it
/// differs from the last build; trimmed by `[doc] versions`; each with the
/// changelog from the one before it.
fn every_version(work_dir: &Path, select: Option<&DocVersions>) -> Result<Vec<Version>> {
    let mut versions = vec![];
    let mut head: Option<(Vec<Vec<FrozenUnit>>, FrozenDocs)> = None;
    if refs::ref_exists(work_dir, refs::main_ref()) {
        let history = history::load(work_dir)?;
        let mut seen = std::collections::BTreeSet::new();
        for entry in &history {
            // Newest first: a version built twice keeps its newest commit.
            if !seen.insert(entry.commit.version.clone()) {
                continue;
            }
            let (version, docs) = frozen(work_dir, entry.hash, &entry.commit)?;
            if head.is_none() {
                head = Some((version.schemas.clone(), docs));
            }
            versions.push(version);
        }
        versions.reverse();
    }

    let spinner = ui::spinner("compiling schemas");
    let outcome = comline_core::package::build::compile_package(work_dir);
    ui::finish_spinner(spinner);
    let context = outcome.map_err(|e| core_err("validation failed", e))?;
    let schemas = doc::frozen_schemas(&context);
    let docs = doc::frozen_docs(&context);
    let changed = match &head {
        Some((head_schemas, head_docs)) => *head_schemas != schemas || *head_docs != docs,
        None => true,
    };
    if changed {
        versions.push(Version {
            id: UNRELEASED.into(),
            release: UNRELEASED.into(),
            kind: VersionKind::Unreleased,
            commit: None,
            model: doc::model(&context, None),
            schemas,
            changelog_from: vec![],
            notes: vec![],
        });
    }

    // Each version's changes since the last release before it, over every
    // version (a trimmed list still measures from the real previous
    // release): a release's are what its changelog shows, a dev build's
    // what its page links to. Compared directly between the two trees, so
    // they're exact, not composed from steps.
    let mut since_release = vec![None; versions.len()];
    let mut last_release: Option<usize> = None;
    for (i, version) in versions.iter().enumerate() {
        if let Some(r) = last_release {
            let span = match version.kind {
                VersionKind::Release => Some(ChangelogSpan::Release),
                // Right after the release, the step already is everything.
                _ if r + 1 == i => None,
                _ => Some(ChangelogSpan::SinceRelease),
            };
            if let Some(span) = span {
                let release = &versions[r];
                let mut log = doc::changelog(&release.id, &release.schemas, &version.id, &version.schemas);
                log.span = span;
                since_release[i] = Some((log, release.commit));
            }
        }
        if version.kind == VersionKind::Release {
            last_release = Some(i);
        }
    }
    let mut versions: Vec<(Version, Option<_>)> = versions.into_iter().zip(since_release).collect();

    if let Some(DocVersions::Select(select)) = select {
        if select.releases_only {
            versions.retain(|(v, _)| v.kind == VersionKind::Release);
        }
        if let Some(since) = &select.since {
            let Some(at) = versions.iter().position(|(v, _)| &v.id == since) else {
                let available: Vec<&str> = versions.iter().map(|(v, _)| v.id.as_str()).collect();
                return Err(miette!(
                    "`[doc] versions` since `{since}`: no such version (available: {})",
                    available.join(", ")
                ));
            };
            versions.drain(..at);
        }
        if let Some(last) = select.last {
            let drop = versions.len().saturating_sub(last);
            versions.drain(..drop);
        }
    }

    // Each version's step: the changelog from the one before it in the
    // list, which the compare page composes. Then its changes since the
    // last release.
    let mut out: Vec<Version> = Vec::with_capacity(versions.len());
    for (mut version, since) in versions {
        if let Some(previous) = out.last() {
            let step = doc::changelog(&previous.id, &previous.schemas, &version.id, &version.schemas);
            version.model.changelog = vec![step];
            version.changelog_from.extend(previous.commit);
        }
        if let Some((log, commit)) = since {
            version.model.changelog.push(log);
            version.changelog_from.extend(commit);
        }
        out.push(version);
    }
    Ok(out)
}

/// `0.3.0-dev.2` leads to the release `0.3.0`; `0.3.0` is one.
fn kind_of(version: &str) -> (VersionKind, String) {
    match version.split_once('-') {
        Some((release, _)) => (VersionKind::Dev, release.to_string()),
        None => (VersionKind::Release, version.to_string()),
    }
}

/// `--version`'s one built version.
fn frozen_version(work_dir: &Path, spec: &str) -> Result<Version> {
    let history = history::load(work_dir)?;
    let commit = history::resolve(&history, spec)?;
    let entry = history
        .iter()
        .find(|e| std::ptr::eq(&e.commit, commit))
        .expect("resolved from this history");
    Ok(frozen(work_dir, entry.hash, commit)?.0)
}

/// A built version's model, from its frozen IR and headers, with notes on
/// what an old version lacks; and its headers, to compare the working
/// tree's with.
fn frozen(work_dir: &Path, hash: Hash, commit: &Commit) -> Result<(Version, FrozenDocs)> {
    let spec = &commit.version;
    let store = ObjectStore::new(work_dir);
    // Every compiled schema: the package's own, its dependencies' and std's.
    let all_schemas = history::load_schemas(&store, commit)?;
    let config = history::load_config(&store, commit)?;
    // A built version's config doesn't always freeze the package's name;
    // the working tree's is the same unless the package was renamed.
    let name = comline_core::package::config::ir::frozen::namespace(&config)
        .map(str::to_string)
        .or_else(|| {
            comline_core::package::build::compile_package_without_fetching(work_dir)
                .ok()
                .map(|context| context.config.name.value.to_string())
        })
        .or_else(|| {
            work_dir
                .canonicalize()
                .ok()?
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
        })
        .ok_or_else(|| miette!("version `{spec}` has no package name"))?;
    let root = store
        .read(&commit.tree)
        .and_then(|bytes| Tree::from_bytes(&bytes))
        .map_err(|e| miette!("failed to read version `{spec}`: {e:#}"))?;
    let headers = doc::read_docs(&store, &root)
        .map_err(|e| miette!("failed to read version `{spec}`'s headers: {e:#}"))?;
    let dependencies = doc::frozen_dependencies(&root);
    let model = doc::model_frozen(
        &name,
        Some(spec.clone()),
        &all_schemas,
        &dependencies,
        headers.clone(),
    );
    let schemas = doc::own_frozen_schemas(&name, &all_schemas, &dependencies);

    let mut notes = vec![];
    if !root.entries.iter().any(|e| e.name == doc::DOCS_ENTRY) {
        notes.push(
            "Built before headers were frozen (core#152): this version has no package or \
             module headers."
                .to_string(),
        );
    }
    let json = serde_json::to_value(&model).into_diagnostic()?;
    let protocol_without_wire = json["modules"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|m| m["units"].as_array().into_iter().flatten())
        .any(|u| u["kind"] == "protocol" && u["wire"].is_null());
    if protocol_without_wire {
        notes.push(
            "Built versions don't record their wire tables yet (core#153): protocols show \
             no wire summary."
                .to_string(),
        );
    }
    let (kind, release) = kind_of(spec);
    Ok((
        Version {
            id: spec.clone(),
            release,
            kind,
            commit: Some(hash),
            model,
            schemas,
            changelog_from: vec![],
            notes,
        },
        headers,
    ))
}

/// Empty `dir` (creating it): a version's folder holds only its render.
fn replace_dir(dir: &Path) -> Result<()> {
    if dir.is_dir() {
        std::fs::remove_dir_all(dir)
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to remove `{}`", dir.display()))?;
    }
    std::fs::create_dir_all(dir)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to create `{}`", dir.display()))
}

fn write_files(dir: &Path, files: &[FileOut]) -> Result<()> {
    for file in files {
        let path = dir.join(file.path_buf());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .into_diagnostic()
                .wrap_err_with(|| format!("failed to create `{}`", parent.display()))?;
        }
        std::fs::write(&path, &file.contents)
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to write `{}`", path.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A built-in renderer's key carries its content-hashed version, so a
    /// CLI with a changed renderer re-renders every folder.
    #[test]
    fn a_changed_renderer_invalidates_every_folder() {
        let renderers = Renderers {
            registry: renderer_registry(),
            external: None,
        };
        for (format, version) in [
            ("html", comline_doc_html::RENDERER.version),
            ("markdown", comline_doc_markdown::RENDERER.version),
        ] {
            assert!(version.contains('+'), "{version}");
            assert!(renderers
                .key(format)
                .contains(&format!("{format}@{version} ")));
        }

        let links = BTreeMap::new();
        let commits: Vec<Hash> = (0..3u8).map(|i| Hash::from_bytes(&[i])).collect();
        let keyed = |renderer: &str| {
            let mut manifest = Manifest::default();
            for (i, commit) in commits.iter().enumerate() {
                let previous: Vec<Hash> = i.checked_sub(1).map(|p| commits[p]).into_iter().collect();
                let key = cache_key(commit, &previous, renderer, &links);
                manifest.versions.insert(format!("v{i}"), Some(key));
            }
            manifest
        };
        let old = keyed("html@0.1.0+0000000000000000 via comline 0.2.0");
        let same = keyed("html@0.1.0+0000000000000000 via comline 0.2.0");
        let changed = keyed("html@0.1.0+0000000000000001 via comline 0.2.0");
        for (id, key) in &same.versions {
            assert!(old.reuses(id, key), "{id} should be kept");
        }
        for (id, key) in &changed.versions {
            assert!(!old.reuses(id, key), "{id} should re-render");
        }
        // The unreleased state is never reused.
        assert!(!old.reuses("unreleased", &None));
    }
}
