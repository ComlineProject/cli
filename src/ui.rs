//! User-facing status output, decoupled from the `tracing` diagnostic layer.
//!
//! `tracing` (`-v` / `-vv`) carries diagnostics coming out of `comline-core`.
//! This module carries the CLI's own progress and result lines: written to
//! **stderr** so stdout stays clean for machine-readable payloads (e.g. `comline
//! completions`). By default it is colored and uses a few leading symbols;
//! `--plain` drops all of that for logs and CI. Coloring otherwise goes through
//! `anstream`, which also strips escapes when stderr is not a terminal or
//! `NO_COLOR` is set.

use std::borrow::Cow;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anstyle::{AnsiColor, Color, Style};
use indicatif::{ProgressBar, ProgressStyle};

static QUIET: AtomicBool = AtomicBool::new(false);
static VERBOSE: AtomicBool = AtomicBool::new(false);
static PLAIN: AtomicBool = AtomicBool::new(false);

/// When set, everything except [`error`] is suppressed.
pub fn set_quiet(quiet: bool) {
    QUIET.store(quiet, Ordering::Relaxed);
}

/// When set, spinners are disabled so they don't fight the `tracing` output the
/// user asked for with `-v`.
pub fn set_verbose(verbose: bool) {
    VERBOSE.store(verbose, Ordering::Relaxed);
}

/// When set, output carries no color, no leading symbols and no progress
/// animation — plain lines suitable for log files and CI.
pub fn set_plain(plain: bool) {
    PLAIN.store(plain, Ordering::Relaxed);
}

fn quiet() -> bool {
    QUIET.load(Ordering::Relaxed)
}

fn verbose() -> bool {
    VERBOSE.load(Ordering::Relaxed)
}

/// Whether `--plain` output is in effect.
pub fn plain() -> bool {
    PLAIN.load(Ordering::Relaxed)
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

/// `" in <path>"` for a header line, or `""` when `p` is the current directory.
///
/// The path is shown relative to the invocation CWD when it lives under it,
/// otherwise as given (a path passed via `--path` is left as typed).
pub fn at_path(p: &Path) -> String {
    let shown = match std::env::current_dir() {
        Ok(cwd) => p
            .strip_prefix(&cwd)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| p.to_path_buf()),
        Err(_) => p.to_path_buf(),
    };
    let shown = shown.display().to_string();
    if shown.is_empty() || shown == "." {
        String::new()
    } else {
        format!(" in {shown}")
    }
}

/// A high-level step is starting, e.g. `• Building project`.
pub fn step(msg: impl AsRef<str>) {
    if quiet() {
        return;
    }
    let msg = msg.as_ref();
    if plain() {
        anstream::eprintln!("{msg}");
    } else {
        anstream::eprintln!("{CYAN}•{CYAN:#} {msg}");
    }
}

/// A step finished successfully, e.g. `✓ Project built`.
pub fn success(msg: impl AsRef<str>) {
    if quiet() {
        return;
    }
    let msg = msg.as_ref();
    if plain() {
        anstream::eprintln!("{msg}");
    } else {
        anstream::eprintln!("{GREEN}✓{GREEN:#} {msg}");
    }
}

/// A section header (e.g. a changelog group title) — bold, or plain text.
pub fn group(msg: impl AsRef<str>) {
    if quiet() {
        return;
    }
    let msg = msg.as_ref();
    if plain() {
        anstream::eprintln!("{msg}");
    } else {
        anstream::eprintln!("{BOLD}{msg}{BOLD:#}");
    }
}

/// An indented detail line under a step.
pub fn detail(msg: impl AsRef<str>) {
    if quiet() {
        return;
    }
    anstream::eprintln!("  {}", msg.as_ref());
}

/// A quiet, purely informational line — dimmed, or plain text.
pub fn note(msg: impl AsRef<str>) {
    if quiet() {
        return;
    }
    let msg = msg.as_ref();
    if plain() {
        anstream::eprintln!("{msg}");
    } else {
        anstream::eprintln!("{DIM}{msg}{DIM:#}");
    }
}

/// A non-fatal warning.
pub fn warn(msg: impl AsRef<str>) {
    if quiet() {
        return;
    }
    let msg = msg.as_ref();
    if plain() {
        anstream::eprintln!("warning: {msg}");
    } else {
        anstream::eprintln!("{YELLOW}warning{YELLOW:#} {msg}");
    }
}

/// A fatal error. Always printed, even under `--quiet`.
pub fn error(msg: impl AsRef<str>) {
    let msg = msg.as_ref();
    if plain() {
        anstream::eprintln!("error: {msg}");
    } else {
        anstream::eprintln!("{RED}error{RED:#} {msg}");
    }
}

/// A steady-tick spinner for a long-running step, drawn on stderr.
///
/// Returns a hidden bar (a no-op) under `--quiet`, `--plain`, `-v`, or when
/// stderr is not a terminal, so callers can use it unconditionally. Pass the
/// returned bar to [`finish_spinner`] when the step ends.
pub fn spinner(msg: impl Into<Cow<'static, str>>) -> ProgressBar {
    if quiet() || verbose() || plain() {
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
