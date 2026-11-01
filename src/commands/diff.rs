//! `comline diff <old> <new>` — changelog between two frozen versions: the
//! package's own schema changes, those within its reach of each dependency,
//! its config changes, and the bump they call for (core's comparison, the
//! same one `build` uses).

use std::path::Path;

use comline_core::package::build::cas::build::compare;
use comline_core::package::build::cas::objects::{Commit, Tree};
use comline_core::package::build::cas::ObjectStore;
use miette::Result;

use crate::commands::ensure_project;
use crate::{changes, history, ui};

pub fn run(work_dir: &Path, old: &str, new: &str) -> Result<()> {
    ensure_project(work_dir)?;

    let chain = history::load(work_dir)?;
    let store = ObjectStore::new(work_dir);

    let old_commit = history::resolve(&chain, old)?;
    let new_commit = history::resolve(&chain, new)?;

    ui::step(format!(
        "Diff {} {} {}",
        label(old, old_commit),
        ui::arrow(),
        label(new, new_commit)
    ));

    if old_commit.tree == new_commit.tree {
        ui::detail("identical — no schema changes");
        return Ok(());
    }

    let tree = |commit: &Commit| -> Result<Tree> {
        let bytes = store
            .read(&commit.tree)
            .map_err(|e| miette::miette!("failed to read root tree: {e:#}"))?;
        Tree::from_bytes(&bytes).map_err(|e| miette::miette!("corrupt root tree: {e:#}"))
    };
    let comparison = compare(&store, &tree(old_commit)?, &tree(new_commit)?)
        .map_err(|e| miette::miette!("failed to compare the versions: {e:#}"))?;
    changes::render(&comparison.schema_changes);
    for dependency in &comparison.dependency_changes {
        ui::group(format!("In dependency `{}`:", dependency.name));
        changes::render(&dependency.changes);
    }
    changes::render_config(&comparison.config_changes);
    ui::detail(changes::bump_line(comparison.bump));
    Ok(())
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
