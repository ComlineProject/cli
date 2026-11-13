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
use comline_codegen::connections;
use comline_core::compat::{self, PackageFacts};
use comline_core::inspect::{self, Acceptance, Choice, ProtocolSummary, Source};
use miette::{IntoDiagnostic, Result};

use crate::commands::{core_err, ensure_project};
use crate::history;
use crate::ui;

const HELD: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Green)));
const GIVEN_UP: Style = Style::new().fg_color(Some(Color::Ansi(AnsiColor::Yellow)));
const DIM: Style = Style::new().dimmed();
const BOLD: Style = Style::new().bold();

pub fn run(work_dir: &Path, json: bool) -> Result<()> {
    ensure_project(work_dir)?;
    history::ensure_readable(work_dir)?;

    let spinner = ui::spinner("compiling schemas");
    let outcome = comline_core::package::build::compile_package(work_dir);
    ui::finish_spinner(spinner);
    let context = outcome.map_err(|e| core_err("validation failed", e))?;

    let protocols = inspect::protocols(&context);
    let unchecked = validators_given_up(work_dir, &context);
    // What the latest build records for `comline@2`: each protocol's line,
    // shape hash and functions' "since". None before the first build.
    let facts = latest_facts(work_dir);
    let mut out = anstream::stdout().lock();
    if json {
        let given_up: Vec<_> = unchecked
            .iter()
            .map(|(target, reason)| serde_json::json!({ "target": target, "reason": reason }))
            .collect();
        let report = serde_json::json!({
            "protocols": protocols,
            "facts": facts,
            "validate_on_decode_given_up": given_up,
        });
        writeln!(
            out,
            "{}",
            serde_json::to_string_pretty(&report).into_diagnostic()?
        )
        .into_diagnostic()?;
        return Ok(());
    }

    let consumer = render_unchecked(&unchecked, ui::plain());
    if protocols.is_empty() {
        ui::note("No protocols to inspect");
        write!(out, "{consumer}").into_diagnostic()?;
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
    if !consumer.is_empty() {
        write!(out, "\n{consumer}").into_diagnostic()?;
    }
    Ok(())
}

/// The targets in comline.toml whose decoding doesn't run `@validators`
/// checks, with the reason each gives (`validate_on_decode = { off = "why" }`).
/// Empty without a comline.toml, or one that doesn't resolve: `generation`
/// reports that.
fn validators_given_up(
    work_dir: &Path,
    context: &comline_core::package::config::ir::context::ProjectContext,
) -> Vec<(String, String)> {
    use comline_codegen::config::{resolve, ComlineToml, Overrides};
    if !work_dir.join(comline_codegen::config::FILE_NAME).exists() {
        return vec![];
    }
    let Ok(cfg) = ComlineToml::load(work_dir) else {
        return vec![];
    };
    let declared = context
        .config_frozen
        .as_deref()
        .map(crate::lang_targets::declared)
        .unwrap_or_default();
    match resolve(&cfg, &declared, work_dir, &Overrides::default()) {
        Ok(targets) => targets
            .into_iter()
            .filter_map(|t| t.validate_on_decode_off.map(|reason| (t.language, reason)))
            .collect(),
        Err(_) => vec![],
    }
}

/// The given-up `validate_on_decode`s, as a consumer's guarantees.
fn render_unchecked(unchecked: &[(String, String)], plain: bool) -> String {
    let mut out = String::new();
    for (target, reason) in unchecked {
        let given_up = if plain {
            "given up".to_string()
        } else {
            format!("{GIVEN_UP}given up{GIVEN_UP:#}")
        };
        out += &format!("target {target} (comline.toml)\n");
        out += &format!("  {:<22} {given_up}\n", "validators on decode");
        out += &format!("      because \"{reason}\"\n");
    }
    out
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
            style(DIM, &source(choice.source, key, &p.framing.value))
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

/// Where a choice came from, for the header. `framing` is the protocol's
/// framing, which a format it implies is named after.
fn source(source: Source, key: &str, framing: &str) -> String {
    match source {
        Source::Annotation => format!("from @{key}"),
        Source::PackageDefault => match key {
            "handshake" => "from config.idp `handshake`".to_string(),
            _ => format!("from config.idp `wire.{key}`"),
        },
        Source::CoreDefault => "core's default".to_string(),
        Source::Framing => format!("{}'s format", connections::framing_title(framing)),
    }
}
