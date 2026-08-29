//! Shared `--watch` loop for `build` and `generate`.

use std::path::Path;
use std::sync::mpsc::channel;
use std::time::Duration;

use miette::{IntoDiagnostic, Result, WrapErr};
use notify_debouncer_mini::notify::RecursiveMode;
use notify_debouncer_mini::{new_debouncer, DebounceEventResult};

use crate::ui;

/// Run `action` once, then again on every debounced change under `<dir>/src` or
/// to `<dir>/config.idp` / `<dir>/comline.toml`, until interrupted (Ctrl-C).
///
/// A failing run is reported but does not stop the loop — the point of `--watch`
/// is to keep going while you fix the error.
pub fn run(work_dir: &Path, label: &str, mut action: impl FnMut() -> Result<()>) -> Result<()> {
    report(label, action());

    let (tx, rx) = channel::<DebounceEventResult>();
    let mut debouncer = new_debouncer(Duration::from_millis(300), tx)
        .into_diagnostic()
        .wrap_err("failed to start file watcher")?;

    let src = work_dir.join("src");
    if src.is_dir() {
        debouncer
            .watcher()
            .watch(&src, RecursiveMode::Recursive)
            .into_diagnostic()
            .wrap_err_with(|| format!("failed to watch `{}`", src.display()))?;
    }
    for name in ["config.idp", "comline.toml"] {
        let file = work_dir.join(name);
        if file.is_file() {
            debouncer
                .watcher()
                .watch(&file, RecursiveMode::NonRecursive)
                .into_diagnostic()
                .wrap_err_with(|| format!("failed to watch `{}`", file.display()))?;
        }
    }

    ui::note(format!(
        "watching {} — press Ctrl-C to stop",
        work_dir.display()
    ));

    for result in rx {
        match result {
            Ok(events) if !events.is_empty() => {
                ui::note("── change detected, re-running ──");
                report(label, action());
            }
            Ok(_) => {}
            Err(e) => ui::warn(format!("watch error: {e}")),
        }
    }
    Ok(())
}

fn report(label: &str, result: Result<()>) {
    if let Err(report) = result {
        ui::error(format!("{label} failed: {report}"));
        for cause in report.chain().skip(1) {
            ui::error(format!("  caused by: {cause}"));
        }
    }
}
