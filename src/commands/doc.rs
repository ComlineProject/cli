//! `comline doc` — the package's documentation (core#145).
//!
//! Core builds the documentation model (`comline_core::doc`): from the
//! compiled working tree, or from a built version's frozen IR and `//!`
//! headers with `--version`. `--changelog-from` adds what changed since an
//! earlier built version. `--format json` writes the model itself; any other
//! format is a renderer's id: `markdown` and `html` are the official ones,
//! linked in like the official generators. `[doc] renderer` in `comline.toml` names an
//! external renderer instead, run exactly like an external generator
//! ([`crate::external`]: the same sandbox, limits and path checks).
//!
//! Like `check`, nothing is written to `.comline/`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use comline_codegen::config::{ComlineToml, DEFAULT_DOC_FORMAT, DEFAULT_DOC_OUT};
use comline_codegen::protocol::{FileOut, RenderOptions, RenderRequest};
use comline_codegen::Registry;
use comline_core::doc::{self, DocModel, DOC_MODEL_VERSION};
use comline_core::package::build::cas::objects::Tree;
use comline_core::package::build::cas::ObjectStore;
use comline_core::schema::ir::frozen::unit::FrozenUnit;
use miette::{miette, IntoDiagnostic, Result, WrapErr};

use crate::commands::{core_err, ensure_project};
use crate::{external, history, ui};

/// The command line's say, each overriding its `[doc]` key.
#[derive(Debug, Default)]
pub struct Flags {
    pub out: Option<PathBuf>,
    pub version: Option<String>,
    pub changelog_from: Option<String>,
    pub format: Option<String>,
}

/// The renderers linked into this binary.
pub(crate) fn renderer_registry() -> Registry {
    let mut registry = Registry::new();
    comline_doc_markdown::register(&mut registry);
    comline_doc_html::register(&mut registry);
    registry
}

pub fn run(work_dir: &Path, flags: Flags) -> Result<()> {
    ensure_project(work_dir)?;
    let cfg = ComlineToml::load(work_dir)?.doc;

    let out = match (&flags.out, &cfg.out) {
        (Some(out), _) => work_dir.join(out),
        (None, Some(out)) => work_dir.join(out),
        (None, None) => work_dir.join(DEFAULT_DOC_OUT),
    };
    let format = flags
        .format
        .or(cfg.format.clone())
        .unwrap_or_else(|| DEFAULT_DOC_FORMAT.to_string());
    let changelog_from = flags.changelog_from.or(cfg.changelog_from.clone());

    let (mut model, schemas) = match &flags.version {
        Some(spec) => frozen_model(work_dir, spec)?,
        None => working_tree_model(work_dir)?,
    };

    if let Some(from) = &changelog_from {
        let history = history::load(work_dir)?;
        let commit = history::resolve(&history, from)?;
        let store = ObjectStore::new(work_dir);
        let old = history::load_schemas(&store, commit)?;
        let to = model
            .package
            .version
            .clone()
            .unwrap_or_else(|| "working tree".to_string());
        model
            .changelog
            .push(doc::changelog(&commit.version, &old, &to, &schemas));
    }

    let files = if format == "json" {
        let json = serde_json::to_string_pretty(&model).into_diagnostic()?;
        vec![FileOut {
            path: "model.json".into(),
            contents: format!("{json}\n").into_bytes(),
        }]
    } else {
        let request = RenderRequest {
            renderer: format.clone(),
            model_version: DOC_MODEL_VERSION,
            model: serde_json::to_string(&model).into_diagnostic()?,
            options: RenderOptions {
                links: cfg.links.clone(),
            },
        };
        match &cfg.renderer {
            Some(path) => {
                let timeout = cfg
                    .timeout
                    .map(Duration::from_secs)
                    .unwrap_or(external::DEFAULT_TIMEOUT);
                let renderer = external::External::open(&work_dir.join(path), timeout)?;
                let description = renderer.describe()?;
                renderer.render(&description, request)?
            }
            None => external::builtin_render(&renderer_registry(), request)?,
        }
    };

    for file in &files {
        let path = out.join(file.path_buf());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .into_diagnostic()
                .wrap_err_with(|| format!("failed to create `{}`", parent.display()))?;
        }
        std::fs::write(&path, &file.contents)
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to write `{}`", path.display()))?;
    }
    ui::success(format!(
        "Documented {} into {} ({} file(s))",
        match &model.package.version {
            Some(v) => format!("{} {v}", model.package.name),
            None => model.package.name.clone(),
        },
        out.display(),
        files.len()
    ));
    Ok(())
}

/// The working tree's model, and its frozen schemas for a changelog.
fn working_tree_model(work_dir: &Path) -> Result<(DocModel, Vec<Vec<FrozenUnit>>)> {
    let spinner = ui::spinner("compiling schemas");
    let outcome = comline_core::package::build::compile_package(work_dir);
    ui::finish_spinner(spinner);
    let context = outcome.map_err(|e| core_err("validation failed", e))?;
    Ok((doc::model(&context, None), doc::frozen_schemas(&context)))
}

/// A built version's model, from its frozen IR and headers.
fn frozen_model(work_dir: &Path, spec: &str) -> Result<(DocModel, Vec<Vec<FrozenUnit>>)> {
    let history = history::load(work_dir)?;
    let commit = history::resolve(&history, spec)?;
    let store = ObjectStore::new(work_dir);
    let schemas = history::load_schemas(&store, commit)?;
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
    let model = doc::model_frozen(&name, Some(commit.version.clone()), &schemas, headers);
    Ok((model, schemas))
}
