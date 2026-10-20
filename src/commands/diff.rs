//! `comline diff <old> <new>` — schema changelog between two frozen versions.

use std::path::Path;

use comline_core::package::build::cas::objects::Commit;
use comline_core::package::build::cas::ObjectStore;
use comline_core::schema::ir::diff::analyze_package_changes;
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

    let old_schemas = history::load_schemas(&store, old_commit)?;
    let new_schemas = history::load_schemas(&store, new_commit)?;
    changes::render(&analyze_package_changes(&old_schemas, &new_schemas));
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
