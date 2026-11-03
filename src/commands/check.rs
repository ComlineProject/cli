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

    let context = outcome.map_err(|e| {
        let detail = format!("{e:#}");
        let unbounded = crate::bounds_migration::Unbounded::of(&detail);
        if unbounded.is_empty() {
            core_err("validation failed", e)
        } else {
            let hint = crate::bounds_migration::hint(&unbounded);
            miette::miette!("validation failed:\n\n{detail}\n{hint}")
        }
    })?;
    for warning in context.rendered_warnings() {
        ui::warn(warning);
    }

    ui::success("Check passed");
    Ok(())
}
