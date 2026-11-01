//! `comline inspect` — what each protocol resolves to on the wire, where
//! each choice came from, and which guarantees it keeps. Read-only, like
//! `check`: nothing is written to `.comline/`.
//!
//! The model is core's (`comline_core::inspect`), shared with the language
//! server's hover. The report goes to stdout; `--json` prints the same as
//! JSON.

use std::io::Write as _;
use std::path::Path;

use anstyle::{AnsiColor, Color, Style};
use comline_core::compat::{self, PackageFacts};
use comline_core::inspect::{self, Acceptance, Choice, ProtocolSummary, Source};
use miette::{IntoDiagnostic, Result};

use crate::commands::{core_err, ensure_project};
use crate::ui;

const HELD: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Green)));
const GIVEN_UP: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Yellow)));
const DIM: Style = Style::new().dimmed();
const BOLD: Style = Style::new().bold();

pub fn run(work_dir: &Path, json: bool) -> Result<()> {
    ensure_project(work_dir)?;

    let spinner = ui::spinner("compiling schemas");
    let outcome = comline_core::package::build::compile_package(work_dir);
    ui::finish_spinner(spinner);
    let context = outcome.map_err(|e| core_err("validation failed", e))?;

    let protocols = inspect::protocols(&context);
    // What the latest build records for `comline@2`: each protocol's line,
    // shape hash and functions' "since". None before the first build.
    let facts = latest_facts(work_dir);
    let mut out = anstream::stdout().lock();
    if json {
        let report = serde_json::json!({ "protocols": protocols, "facts": facts });
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&report).into_diagnostic()?
        )
        .into_diagnostic()?;
        return Ok(());
    }

    if protocols.is_empty() {
        ui::note("No protocols to inspect");
        return Ok(());
    }
    let text = protocols
        .iter()
        .map(|p| {
            let mut text = render(p, ui::plain());
            if !p.dependency {
                if let Some(facts) = &facts {
                    text += &render_facts(
                        facts,
                        &compat::protocol_path(&p.schema, &p.name),
                        ui::plain(),
                    );
                }
            }
            text
        })
        .collect::<Vec<_>>()
        .join("\n");
    write!(out, "{text}").into_diagnostic()?;
    Ok(())
}

/// The latest build's facts, if the package was built. Read-only: the
/// store is never created here.
fn latest_facts(work_dir: &Path) -> Option<PackageFacts> {
    use comline_core::package::build::cas::{main_ref, ref_exists};
    if !ref_exists(work_dir, main_ref()) {
        return None;
    }
    compat::facts(work_dir, None).ok()
}

/// A protocol's line, shape hash and functions' "since", as the latest
/// build records them. `path` is its `namespace::Protocol`: facts are kept
/// per qualified path, so two protocols of one name stay apart.
pub fn render_facts(facts: &PackageFacts, path: &str, plain: bool) -> String {
    let Some(p) = facts.protocols.iter().find(|p| p.path == path) else {
        return String::new();
    };
    let dim = |text: &str| {
        if plain {
            text.to_string()
        } else {
            format!("{DIM}{text}{DIM:#}")
        }
    };
    let mut out = format!(
        "\n  {}\n",
        dim(&format!("{} as of build {}", p.path, facts.version))
    );
    out += &format!("  {:<10} {}\n", "line", p.line);
    out += &format!("  {:<10} {}\n", "shape", p.shape_hash);
    for f in &p.functions {
        out += &format!("  {:<10} {:<24} {}\n", "since", f.name, f.since);
    }
    out
}

/// One protocol: a header of choices, then a row per guarantee. Fits 80
/// columns: anything long goes on its own indented line.
pub fn render(p: &ProtocolSummary, plain: bool) -> String {
    let style = |s: Style, text: &str| {
        if plain {
            text.to_string()
        } else {
            format!("{s}{text}{s:#}")
        }
    };
    let mut out = String::new();

    let whose = if p.dependency {
        format!("dependency {}", p.package)
    } else {
        p.package.clone()
    };
    out += &format!(
        "{} {}\n",
        style(BOLD, &format!("protocol {}::{}", p.schema, p.name)),
        style(DIM, &format!("({whose})"))
    );
    for (key, choice) in [
        ("format", &p.format),
        ("framing", &p.framing),
        ("handshake", &p.handshake),
    ] {
        out += &format!(
            "  {key:<10} {:<24} {}\n",
            choice_value(choice),
            style(DIM, &source(choice.source, key))
        );
    }
    let reason = p.guarantees.iter().find_map(|g| g.reason.as_deref());
    if p.unverified {
        match reason {
            Some(reason) => out += &format!("  {:<10} \"{reason}\"\n", "unverified"),
            None => out += &format!("  {:<10} (no reason given)\n", "unverified"),
        }
    }

    out += "\n";
    for row in &p.guarantees {
        let (mark, state) = match (row.held, plain) {
            (true, true) => ("", style(HELD, "held")),
            (true, false) => ("✓ ", style(HELD, "held")),
            (false, true) => ("", style(GIVEN_UP, "given up")),
            (false, false) => ("✗ ", style(GIVEN_UP, "given up")),
        };
        out += &format!("  {mark}{:<22} {state}\n", row.guarantee.describe());
        for cause in &row.causes {
            out += &format!("      because {cause}\n");
        }
        match &row.acceptance {
            Acceptance::NotNeeded => {}
            Acceptance::Unknown => {
                out += &format!(
                    "      {}\n",
                    style(DIM, "acceptance: up to each consumer's config.idp")
                );
            }
            Acceptance::Accepted(by) => out += &format!("      accepted by {by}\n"),
            Acceptance::NotAccepted => {
                out += &format!("      {}\n", style(GIVEN_UP, "not accepted"))
            }
        }
    }
    out
}

/// `jsonrpc (method_names = qualified)`.
fn choice_value(choice: &Choice) -> String {
    if choice.params.is_empty() {
        return choice.value.clone();
    }
    let params: Vec<String> = choice
        .params
        .iter()
        .map(|(k, v)| format!("{k} = {v}"))
        .collect();
    format!("{} ({})", choice.value, params.join(", "))
}

/// Where a choice came from, for the header.
fn source(source: Source, key: &str) -> String {
    match source {
        Source::Annotation => format!("from @{key}"),
        Source::PackageDefault => match key {
            "handshake" => "from config.idp `handshake`".to_string(),
            _ => format!("from config.idp `wire.{key}`"),
        },
        Source::CoreDefault => "core's default".to_string(),
        Source::Framing => "JSON-RPC's format".to_string(),
    }
}
