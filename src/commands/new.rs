//! `comline new` — scaffold a new project directory.

use std::path::Path;
use std::process::Command;

use miette::{IntoDiagnostic, Result, WrapErr};

use crate::ui;

const MAIN_IDS: &str = "\
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
    write(&root.join("src/main.ids"), MAIN_IDS)?;
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

fn config_idp(name: &str) -> String {
    format!(
        "\
congregation {name}
specification_version = 1

code_generation = {{
    languages = {{
        rust#1.70.0 = {{ package_versions=[all] }}
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
