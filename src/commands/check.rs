//! `comline check` — validate the project without freezing a version.

use std::path::Path;

use miette::Result;

use crate::commands::{core_err, ensure_project};
use crate::ui;

pub fn run(work_dir: &Path) -> Result<()> {
    ensure_project(work_dir)?;

    ui::step(format!("Validating project{}", ui::at_path(work_dir)));
    let spinner = ui::spinner("compiling schemas");
    let outcome = comline_core::package::build::compile_package(work_dir);
    ui::finish_spinner(spinner);

    outcome.map_err(|e| core_err("validation failed", e))?;

    ui::success("Check passed");
    Ok(())
}
