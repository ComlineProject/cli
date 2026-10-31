//! `comline conformance --impl "<command line>"`: an encoding's
//! implementation, run against the vectors its package ships, the way the
//! official runtimes are proven against the official corpus.
//!
//! **Where the vectors are:** an encoding declared in `config.idp`'s
//! `encodings` with `profile = "profiles/lines.json"` ships its vectors
//! beside the profile, at `profiles/lines.vectors.json`: the profile's path
//! with `.json` replaced by `.vectors.json`. The file is a corpus vectors
//! file (`{"feature": ..., "cases": [...]}`), in the conformance spec's case
//! format (ComlineProject/runtime `runtime/conformance/README.md`). Every
//! case in it is run, and the profile's `proven_by` names cases in it.
//!
//! **The official encodings** (`packed@1`, ...) aren't declared by a package
//! and their vectors live in the runtime's corpus: `--corpus <dir>` runs a
//! corpus laid out like `runtime/conformance` (`profiles/<name>.json`,
//! `vectors/*.json`) instead of the package's declarations.
//!
//! **The implementation** is a process speaking the ops protocol, as
//! `comline-reference` does: request lines on stdin, a reply line each on
//! stdout, exit when stdin ends. It's run once, through the same runner as
//! external generators ([`crate::external::native`]): a cleared
//! environment, a time limit and an output cap. Every question goes in up
//! front, since a case's questions never depend on the answers; the first
//! line asks `version`, and a protocol other than the runner's is refused.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use comline_core::package::config::idl::grammar::{Congregation, Key, Value};
use comline_core::package::config::ir::frozen::FrozenUnit;
use comline_runtime_conformance::runner::{
    self, judge, parse_reply, questions, Corpus, FactProof, OPS_PROTOCOL,
};
use comline_runtime_conformance::{Answer, Case, Outcome, Profile};
use miette::{miette, Result};
use serde_json::{json, Value as Json};

use crate::commands::{core_err, ensure_project};
use crate::external::{native, DEFAULT_TIMEOUT};
use crate::ui;

/// One encoding to prove: its profile and the cases that bear on it.
struct Subject {
    profile: Profile,
    cases: Vec<Case>,
}

pub fn run(
    work_dir: &Path,
    implementation: &str,
    encodings: &[String],
    corpus: Option<&Path>,
    timeout: Option<u64>,
) -> Result<()> {
    let subjects = match corpus {
        Some(dir) => from_corpus(dir, encodings)?,
        None => from_package(work_dir, encodings)?,
    };
    let timeout = timeout.map_or(DEFAULT_TIMEOUT, Duration::from_secs);

    let words: Vec<String> = implementation
        .split_whitespace()
        .map(str::to_string)
        .collect();
    let Some((program, args)) = words.split_first() else {
        return Err(miette!(
            "`--impl` is empty: give the implementation's command line"
        ));
    };
    let program = resolve(program)?;
    let name = program
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| program.display().to_string());

    // Line 0 asks `version`; line 1 + i asks case i's questions, as a batch.
    let all: Vec<&Case> = subjects.iter().flat_map(|s| &s.cases).collect();
    let mut input = json!({ "id": "version", "op": "version" }).to_string();
    input.push('\n');
    for (i, case) in all.iter().enumerate() {
        let batch: Vec<Json> = questions(case)
            .iter()
            .enumerate()
            .map(|(j, q)| q.request(json!(format!("{i}.{j}"))))
            .collect();
        input.push_str(&Json::Array(batch).to_string());
        input.push('\n');
    }

    ui::step(format!(
        "Running `{implementation}` against {} case(s) of {} encoding(s)",
        all.len(),
        subjects.len()
    ));
    let output = native::run(&program, args, &name, input.as_bytes(), timeout).map_err(|e| {
        if e.starts_with("didn't finish") {
            miette!(
                "`{name}` didn't answer within {}s and was stopped (raise it with `--timeout`)",
                timeout.as_secs_f64()
            )
        } else {
            miette!("`{name}` {e}")
        }
    })?;
    let output =
        String::from_utf8(output).map_err(|_| miette!("`{name}` wrote output that isn't UTF-8"))?;
    let mut lines = output.lines();

    let version = lines
        .next()
        .and_then(|l| serde_json::from_str::<Json>(l).ok())
        .ok_or_else(|| miette!("`{name}` didn't answer `version`"))?;
    let version = &version["version"];
    if version["protocol"] != json!(OPS_PROTOCOL) {
        return Err(miette!(
            "`{name}` speaks ops protocol {}, but this runner speaks {OPS_PROTOCOL}",
            version["protocol"]
        ));
    }
    let claimed = |name: &str| {
        ["formats", "framings", "handshakes"].iter().any(|key| {
            version[key]
                .as_array()
                .is_some_and(|names| names.iter().any(|n| n == name))
        })
    };

    let mut outcomes = BTreeMap::new();
    for (i, case) in all.iter().enumerate() {
        let outcome = match lines.next() {
            None => Outcome::Fail("no reply line".into()),
            Some(line) => answers(line, i).map_or_else(Outcome::Fail, |a| judge(case, &a)),
        };
        outcomes.insert(case.id.clone(), outcome);
    }

    let mut failures = 0;
    for subject in &subjects {
        failures += report(subject, &outcomes, claimed(&subject.profile.name));
    }
    if failures > 0 {
        return Err(miette!("conformance failed: {failures} problem(s)"));
    }
    ui::success(format!(
        "`{name}` conforms to every encoding it was run against"
    ));
    Ok(())
}

/// The answers on one reply line: the batch for case `i`.
fn answers(line: &str, i: usize) -> std::result::Result<Vec<Answer>, String> {
    let reply: Json =
        serde_json::from_str(line).map_err(|e| format!("a reply that isn't JSON: {e}"))?;
    let Json::Array(replies) = reply else {
        return Err(format!("a batch got a reply that isn't an array: {reply}"));
    };
    replies
        .iter()
        .enumerate()
        .map(|(j, r)| {
            let (id, answer) = parse_reply(r)?;
            if id != json!(format!("{i}.{j}")) {
                return Err(format!("reply {j} has id {id}"));
            }
            Ok(answer)
        })
        .collect()
}

/// Prints one encoding's results; the number of problems.
fn report(subject: &Subject, outcomes: &BTreeMap<String, Outcome>, claimed: bool) -> usize {
    let profile = &subject.profile;
    let kind = serde_json::to_value(profile.kind).unwrap_or_default();
    let passed = subject
        .cases
        .iter()
        .filter(|c| outcomes.get(&c.id) == Some(&Outcome::Pass))
        .count();
    ui::group(format!(
        "{} ({}): {passed}/{} case(s) pass",
        profile.name,
        kind.as_str().unwrap_or_default(),
        subject.cases.len()
    ));
    let mut problems = 0;
    if !claimed {
        ui::note(format!(
            "the implementation's `version` doesn't list `{}`",
            profile.name
        ));
    }
    let case = |id: &str| subject.cases.iter().find(|c| c.id == id);
    for FactProof {
        fact,
        value,
        result,
    } in runner::prove(profile, case, outcomes)
    {
        match result {
            Ok(()) => ui::detail(format!("proven  {fact}: {value}")),
            Err(why) => {
                problems += 1;
                ui::error(format!("unproven  {fact}: {value} — {why}"));
            }
        }
    }
    for case in &subject.cases {
        match &outcomes[&case.id] {
            Outcome::Pass => {}
            Outcome::Fail(why) => {
                problems += 1;
                ui::error(format!("{} failed: {}", case.id, indent(why)));
            }
            Outcome::NotImplemented(what) => {
                problems += 1;
                ui::error(format!("{} not implemented: {what}", case.id));
            }
        }
    }
    problems
}

fn indent(text: &str) -> String {
    let mut out = String::new();
    for (i, line) in text.lines().enumerate() {
        if i > 0 {
            out.push_str("\n    ");
        }
        let _ = write!(out, "{line}");
    }
    out
}

/// A program as a shell would find it: a path as given, or a bare name on
/// `PATH`. The implementation runs with a cleared environment, so the name
/// is resolved here.
fn resolve(program: &str) -> Result<PathBuf> {
    let path = Path::new(program);
    if path.components().count() > 1 || path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
        .ok_or_else(|| miette!("`{program}` isn't on PATH: give its path"))
}

/// The encodings in a runtime-style corpus: its profiles (all, or those
/// named), each with the cases of the features its facts are proven from.
fn from_corpus(dir: &Path, names: &[String]) -> Result<Vec<Subject>> {
    let corpus: Corpus =
        runner::load_corpus(dir).map_err(|e| miette!("loading the corpus: {e}"))?;
    if corpus.profiles.is_empty() {
        return Err(miette!("{} has no profiles/*.json", dir.display()));
    }
    let chosen: Vec<&Profile> = if names.is_empty() {
        corpus.profiles.values().collect()
    } else {
        names
            .iter()
            .map(|n| {
                corpus
                    .profiles
                    .get(n)
                    .ok_or_else(|| miette!("the corpus has no profile for `{n}`"))
            })
            .collect::<Result<_>>()?
    };
    Ok(chosen
        .into_iter()
        .map(|profile| Subject {
            cases: corpus.cases_for(profile).into_iter().cloned().collect(),
            profile: profile.clone(),
        })
        .collect())
}

/// The encodings the package declares (all, or those named), each with its
/// profile and the vectors beside it.
fn from_package(work_dir: &Path, names: &[String]) -> Result<Vec<Subject>> {
    ensure_project(work_dir)?;
    let context = comline_core::package::build::compile_package(work_dir)
        .map_err(|e| core_err("the package doesn't compile", e))?;
    let declared: Vec<String> = context
        .config_frozen
        .iter()
        .flatten()
        .filter_map(|unit| match unit {
            FrozenUnit::Encoding { name, .. } => Some(name.clone()),
            _ => None,
        })
        .collect();
    let paths = profile_paths(&context.config);

    let chosen: Vec<String> = if names.is_empty() {
        if declared.is_empty() {
            return Err(miette!(
                "the package declares no encodings (`encodings` in config.idp); to run an official \
                 encoding's vectors, pass `--corpus <runtime/conformance>`"
            ));
        }
        declared
    } else {
        for n in names {
            if !declared.contains(n) {
                let hint = if comline_core::encodings::is_bare(n) {
                    "; an official encoding's vectors are in the runtime's corpus: pass \
                     `--corpus <runtime/conformance>`"
                } else {
                    ""
                };
                return Err(miette!("the package doesn't declare `{n}`{hint}"));
            }
        }
        names.to_vec()
    };

    chosen
        .into_iter()
        .map(|name| {
            let profile_path = paths
                .get(&name)
                .ok_or_else(|| miette!("can't find `{name}`'s profile in config.idp"))?;
            let profile_path = work_dir.join(profile_path);
            let profile = runner::load_profile(&profile_path).map_err(|e| miette!("{e}"))?;
            let vectors_path = vectors_beside(&profile_path);
            if !vectors_path.exists() {
                return Err(miette!(
                    "`{name}` has no vectors: expected {} beside its profile",
                    vectors_path.display()
                ));
            }
            let vectors = runner::load_vectors(&vectors_path).map_err(|e| miette!("{e}"))?;
            Ok(Subject {
                profile,
                cases: vectors.cases,
            })
        })
        .collect()
}

/// Where an encoding's vectors are: its profile's path with `.json`
/// replaced by `.vectors.json` (`profiles/lines.json` →
/// `profiles/lines.vectors.json`).
pub fn vectors_beside(profile: &Path) -> PathBuf {
    let stem = profile
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    profile.with_file_name(format!("{stem}.vectors.json"))
}

/// Each declared encoding's `profile` path, as `config.idp` writes it. Core
/// has already checked every entry.
fn profile_paths(config: &Congregation) -> BTreeMap<String, String> {
    let mut paths = BTreeMap::new();
    for top in &config.assignments {
        if top.value.key.text() != "encodings" {
            continue;
        }
        let Value::Dictionary(entries) = &top.value.value else {
            continue;
        };
        for entry in &entries.assignments {
            let Value::Dictionary(fields) = &entry.value.value else {
                continue;
            };
            for field in &fields.assignments {
                if let (Key::Identifier(k), Value::String(s)) =
                    (&field.value.key, &field.value.value)
                {
                    if k.value == "profile" {
                        paths.insert(entry.value.key.text(), unquote(&s.value));
                    }
                }
            }
        }
    }
    paths
}

fn unquote(s: &str) -> String {
    s.strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(s)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vectors_sit_beside_the_profile() {
        assert_eq!(
            vectors_beside(Path::new("/p/profiles/lines.json")),
            Path::new("/p/profiles/lines.vectors.json")
        );
        assert_eq!(
            vectors_beside(Path::new("x@1.json")),
            Path::new("x@1.vectors.json")
        );
    }
}
