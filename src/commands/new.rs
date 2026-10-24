//! `comline new` — scaffold a new project directory.

use std::path::Path;
use std::process::Command;

use miette::{IntoDiagnostic, Result, WrapErr};

use crate::commands::generate::generator_registry;
use crate::ui;

const GREETING_IDS: &str = "\
/// The language a greeting is written in.
enum Language {
    English
    Spanish
    Japanese
}

struct Greeting {
    message: string
    language: Language
}
";

const GITIGNORE: &str = "\
# Comline build artifacts
.comline/
";

/// Consumer-owned code-generation config. Committed. All-commented by default —
/// Which languages get generated always comes from `config.idp`; this file
/// only says where each lands. Everything here is commented by default, so a
/// new project writes to `generated/{{language}}/{{namespace}}.{{ext}}`.
const COMLINE_TOML: &str = "\
# comline.toml — where `comline generate` writes code, and how.
#
# Which languages are generated comes from `config.idp`'s
# `code_generation.languages` — this file says where each one lands.
# With everything below commented out they all land in `generated/`.
#
# [generate]
# only = [\"rust\"]                                  # generate just these, out of the declared languages
# out = \"generated\"                                # output root
# layout = \"{{language}}/{{namespace}}.{{ext}}\"    # path under the root
# mode = \"code\"                                    # code (loose source) | lib (a buildable package); dylib not built yet
# external_std = false                             # reference `comline_std` rather than inlining std; defaults on in `lib`
# package_versions = \"latest\"                      # latest | all | [\"0.3.0\", \"0.4.0\"]
# default_framing = \"jsonrpc\"                      # wire framing for protocols with no `@framing` (else datagram)
# validate_on_decode = \"off\"                      # run `@validators` while decoding: off | collect | first
# async = false                                    # protocols on a connection get async handlers and senders
#
# One block per declared language you want to configure. A block sets that
# language's output; it does not decide which languages are generated, and
# naming one `config.idp` doesn't declare is an error.
# [[generate.target]]
# language = \"rust\"
# out = \"src/generated\"
";

pub fn run(work_dir: &Path, name: &str, git: bool) -> Result<()> {
    let root = work_dir.join(name);
    if root.exists() {
        return Err(miette::miette!("`{}` already exists", root.display()));
    }

    let package = package_ident(name);

    ui::step(format!("Creating project {name}"));
    if package != name {
        ui::note(format!(
            "package name is `{package}` (the directory is `{name}`; \
             Comline names must be letters, digits and `_`)"
        ));
    }

    std::fs::create_dir_all(root.join("src"))
        .into_diagnostic()
        .wrap_err("failed to create project directories")?;

    write(&root.join("config.idp"), &config_idp(&package))?;
    write(&root.join("comline.toml"), COMLINE_TOML)?;
    write(&root.join("src/greeting.ids"), GREETING_IDS)?;
    write(&root.join(".gitignore"), GITIGNORE)?;

    if git {
        git_init(&root);
    }

    ui::success(format!("Created {}", root.display()));
    ui::detail(format!("next: cd {name} && comline build"));
    Ok(())
}

/// Turn a project name into a valid Comline identifier (`[A-Za-z_][A-Za-z0-9_]*`)
/// for use as the `congregation` name — the directory keeps the original name.
fn package_ident(name: &str) -> String {
    let mut ident: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if ident.is_empty() || ident.starts_with(|c: char| c.is_ascii_digit()) {
        ident.insert(0, '_');
    }
    ident
}

/// The language a new project declares: Rust at its generator's floor, the
/// oldest toolchain it generates for. That's the widest promise, and a
/// consumer can raise a version but never lower it, so a higher one would
/// shut older toolchains out for good. A binary built without Rust
/// scaffolds the first language it has.
fn scaffold_language() -> String {
    let targets = generator_registry().targets();
    targets
        .iter()
        .find(|t| t.name == "rust")
        .or(targets.first())
        .map(|t| format!("{}#{}", t.name, t.version))
        .unwrap_or_else(|| "rust#1.85.0".to_string())
}

fn config_idp(name: &str) -> String {
    let language = scaffold_language();
    format!(
        "\
congregation {name}
specification_version = 1

code_generation = {{
    languages = {{
        {language} = {{}}
    }}
}}
"
    )
}

fn write(path: &Path, contents: &str) -> Result<()> {
    std::fs::write(path, contents)
        .into_diagnostic()
        .wrap_err_with(|| format!("failed to write `{}`", path.display()))
}

fn git_init(root: &Path) {
    match Command::new("git")
        .arg("init")
        .arg("-q")
        .current_dir(root)
        .status()
    {
        Ok(status) if status.success() => ui::detail("initialized a git repository"),
        Ok(_) => ui::warn("`git init` exited with an error; skipping"),
        Err(e) => ui::warn(format!("could not run `git init`: {e}")),
    }
}
