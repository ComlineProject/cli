//! User-facing status output, decoupled from the `tracing` diagnostic layer.
//!
//! `tracing` (`-v` / `-vv`) carries diagnostics coming out of `comline-core`.
//! This module carries the CLI's own progress and result lines: colored, concise,
//! written to **stderr** so stdout stays clean for machine-readable payloads
//! (e.g. `comline completions`). Coloring goes through `anstream`, which strips
//! escapes automatically when stderr is not a terminal or `NO_COLOR` is set.

use std::borrow::Cow;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anstyle::{AnsiColor, Color, Style};
use indicatif::{ProgressBar, ProgressStyle};

static QUIET: AtomicBool = AtomicBool::new(false);
static VERBOSE: AtomicBool = AtomicBool::new(false);

/// When set, everything except [`error`] is suppressed.
pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

/// When set, spinners are disabled so they don't fight the `tracing` output the
/// user asked for with `-v`.
pub fn set_verbose(verbose: bool) {
    VERBOSE.store(verbose, Ordering::Relaxed);
}

fn quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

const GREEN: Style = Style::new()
    .fg_color(Some(Color::Ansi(AnsiColor::Green)))
    .bold();
const RED: Style = Style::new()
    .fg_color(Some(Color::Ansi(AnsiColor::Red)))
    .bold();
const YELLOW: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Yellow)));
const CYAN: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Cyan)));
const DIM: Style = Style::new().dimmed();
const BOLD: Style = Style::new().bold();

/// A high-level step is starting, e.g. `• Building project`.
pub fn step(msg: impl AsRef<str>) {
    if quiet() {
        return;
    }
    anstream::eprintln!("{CYAN}•{CYAN:#} {}", msg.as_ref());
}

/// A step finished successfully, e.g. `✓ Project built`.
pub fn success(msg: impl AsRef<str>) {
    if quiet() {
        return;
    }
    anstream::eprintln!("{GREEN}✓{GREEN:#} {}", msg.as_ref());
}

/// A bold, symbol-free section header (e.g. a changelog group title).
pub fn group(msg: impl AsRef<str>) {
    if quiet() {
        return;
    }
    anstream::eprintln!("{BOLD}{}{BOLD:#}", msg.as_ref());
}

/// An indented detail line under a step.
pub fn detail(msg: impl AsRef<str>) {
    if quiet() {
        return;
    }
    anstream::eprintln!("  {}", msg.as_ref());
}

/// A dimmed, purely informational line.
pub fn note(msg: impl AsRef<str>) {
    if quiet() {
        return;
    }
    anstream::eprintln!("{DIM}{}{DIM:#}", msg.as_ref());
}

/// A non-fatal warning.
pub fn warn(msg: impl AsRef<str>) {
    if quiet() {
        return;
    }
    anstream::eprintln!("{YELLOW}warning{YELLOW:#} {}", msg.as_ref());
}

/// A fatal error. Always printed, even under `--quiet`.
pub fn error(msg: impl AsRef<str>) {
    anstream::eprintln!("{RED}error{RED:#} {}", msg.as_ref());
}

/// A steady-tick spinner for a long-running step, drawn on stderr.
///
/// Returns a hidden bar (a no-op) under `--quiet`, under `-v`, or when stderr is
/// not a terminal, so callers can use it unconditionally. Pass the returned bar
/// to [`finish_spinner`] when the step ends.
pub fn spinner(msg: impl Into<Cow<'static, str>>) -> ProgressBar {
    if quiet() || verbose() {
        return ProgressBar::hidden();
    }
    let pb = ProgressBar::new_spinner();
    if let Ok(style) = ProgressStyle::with_template("{spinner:.cyan} {msg}") {
        pb.set_style(style);
    }
    pb.enable_steady_tick(Duration::from_millis(90));
    pb.set_message(msg);
    pb
}

/// Clear a spinner created by [`spinner`].
pub fn finish_spinner(pb: ProgressBar) {
    pb.finish_and_clear();
}
