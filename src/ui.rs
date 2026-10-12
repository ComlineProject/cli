//! User-facing status output, decoupled from the `tracing` diagnostic layer.
//!
//! `tracing` (`-v` / `-vv`) carries diagnostics coming out of `comline-core`.
//! This module carries the CLI's own progress and result lines: written to
//! **stderr** so stdout stays clean for machine-readable payloads (e.g.
//! `comline completions`, `comline targets`). By default it is colored and
//! uses a few leading symbols; `--plain` drops all of that for logs and CI.
//! Coloring otherwise goes through `anstream`, which also strips escapes when
//! stderr is not a terminal or `NO_COLOR` is set.

use std::borrow::Cow;
use std::path::{Path, PathBuf};
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

/// Whether `-v` was given: `comline-core`'s diagnostics are wanted.
pub fn verbose() -> bool {
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

/// `" (in <path>/)"` for a header line, or `""` when `p` is the current directory.
///
/// The path is shown relative to the invocation CWD when it lives under it,
/// otherwise as given (a path passed via `--path` is left as typed). A trailing
/// slash marks it as a directory.
pub fn at_path(p: &Path) -> String {
    let shown = match std::env::current_dir() {
        Ok(cwd) => p
            .strip_prefix(&cwd)
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| p.to_path_buf()),
        Err(_) => p.to_path_buf(),
    };
    let mut shown = shown.display().to_string();
    if shown.is_empty() || shown == "." {
        return String::new();
    }
    if !shown.ends_with('/') {
        shown.push('/');
    }
    format!(" (in {shown})")
}

/// `"→"` normally, `"->"` under `--plain`. Used for schema/version mappings.
pub fn arrow() -> &'static str {
    if plain() {
        "->"
    } else {
        "→"
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

/// Print `paths` as a directory tree, rooted at their deepest common
/// ancestor — what `generate --dry-run` shows in place of the file-by-file
/// mapping a real run prints.
///
/// A tree rather than a flat list because `lib` mode's output is a package:
/// its shape (a manifest beside a `src/` of modules) is most of what there
/// is to check before letting it write, often to a directory outside the
/// project. Paths are shown relative to `base` when they live under it,
/// absolute otherwise — the same rule [`at_path`] follows.
pub fn tree(base: &Path, paths: &[PathBuf]) {
    if quiet() || paths.is_empty() {
        return;
    }

    let root = common_root(paths);
    let shown_root = root.strip_prefix(base).unwrap_or(&root);
    let mut label = if shown_root.as_os_str().is_empty() {
        ".".to_string()
    } else {
        shown_root.display().to_string()
    };
    label.push('/');
    detail(format!("     {label}"));

    // One entry per line of the tree, as the `/`-separated segments below
    // `root`. Sorted so directories group and the output is stable.
    let mut rows: Vec<Vec<String>> = paths
        .iter()
        .filter_map(|p| p.strip_prefix(&root).ok())
        .map(|rel| {
            rel.components()
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
                .collect()
        })
        .collect();
    rows.sort();
    rows.dedup();

    print_level(&rows, &[], "     ");
}

/// The deepest directory containing every path in `paths`.
fn common_root(paths: &[PathBuf]) -> PathBuf {
    let mut components: Option<Vec<std::ffi::OsString>> = None;
    for path in paths {
        let parent: Vec<std::ffi::OsString> = path
            .parent()
            .unwrap_or(Path::new(""))
            .components()
            .map(|c| c.as_os_str().to_os_string())
            .collect();
        components = Some(match components {
            None => parent,
            Some(common) => common
                .into_iter()
                .zip(parent)
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| a)
                .collect(),
        });
    }
    components.unwrap_or_default().into_iter().collect()
}

/// Render the children of `prefix` (a path of segments already drawn),
/// recursing a level at a time. `indent` is the running left margin, which
/// grows by either a pipe or blank depending on whether the parent was the
/// last of its siblings.
fn print_level(rows: &[Vec<String>], prefix: &[String], indent: &str) {
    let mut children: Vec<&String> = rows
        .iter()
        .filter(|r| r.len() > prefix.len() && r.starts_with(prefix))
        .map(|r| &r[prefix.len()])
        .collect();
    children.dedup();

    let (tee, elbow, pipe) = if plain() {
        ("|- ", "\\- ", "|  ")
    } else {
        ("├─ ", "└─ ", "│  ")
    };

    for (i, name) in children.iter().enumerate() {
        let last = i + 1 == children.len();
        let mut path = prefix.to_vec();
        path.push((*name).clone());

        // A name with nothing below it is a file; anything else is a
        // directory, and gets the trailing slash that says so.
        let is_dir = rows
            .iter()
            .any(|r| r.len() > path.len() && r.starts_with(&path));
        let branch = if last { elbow } else { tee };
        detail(format!(
            "{indent}{branch}{name}{}",
            if is_dir { "/" } else { "" },
        ));

        if is_dir {
            let deeper = format!("{indent}{}", if last { "   " } else { pipe });
            print_level(rows, &path, &deeper);
        }
    }
}
