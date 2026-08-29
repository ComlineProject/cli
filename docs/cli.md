# comline CLI guide

`comline` is the command-line interface to [Comline](https://github.com/ComlineProject).
It drives the everyday workflow around Comline schemas: scaffold a project,
validate it, freeze immutable versions, compare versions, and generate code — all
on top of `comline-core`.

This is the long-form reference. For a two-minute tour see the
[README](../README.md).

- [Overview](#overview)
- [Installation](#installation)
- [Project layout](#project-layout)
- [Versioning model](#versioning-model)
- [Commands](#commands)
  - [`new`](#comline-new)
  - [`check`](#comline-check)
  - [`build`](#comline-build)
  - [`generate`](#comline-generate)
  - [`diff`](#comline-diff)
  - [`clean`](#comline-clean)
  - [`completions`](#comline-completions)
- [Global flags](#global-flags)
- [Output and exit codes](#output-and-exit-codes)
- [Watch mode](#watch-mode)
- [Shell completions and man pages](#shell-completions-and-man-pages)
- [Editor and pre-commit integration](#editor-and-pre-commit-integration)

## Overview

A Comline project is a directory with a package manifest (`config.idp`) and one
or more schema files (`src/**/*.ids`), optionally a `comline.toml` for
code-generation output. Running a command takes those inputs through:

**parse → resolve imports → validate → (freeze into `.comline/`) → (generate code)**

`check` stops after validation. `build` adds the freeze step and records a new
version. `generate` stops after validation too — it does **not** freeze — and
writes generated code per `comline.toml`. `diff` reads two already-frozen
versions and reports what changed between them.

## Installation

`comline` is not published to crates.io yet. Build it from source:

```bash
git clone https://github.com/ComlineProject/cli
cd cli
cargo install --path .
```

This installs the `comline` binary into `~/.cargo/bin`. Man pages and shell
completions are generated during the build — see
[Shell completions and man pages](#shell-completions-and-man-pages).

## Project layout

`comline new my-api` creates:

```
my-api/
├── config.idp        # package manifest: name, spec version, code_generation targets
├── comline.toml      # where `generate` writes code and how (all-commented by default)
├── src/
│   └── main.ids      # a sample enum + struct to build on
└── .gitignore        # ignores .comline/
```

`config.idp` starts as:

```
congregation my_api
specification_version = 1

code_generation = {
    languages = {
        rust#1.70.0 = { package_versions=[all] }
    }
}
```

The directory keeps the name you gave; the `congregation` name is that name
reduced to a valid Comline identifier (letters, digits and `_`), so
`comline new my-api` produces the directory `my-api/` with `congregation my_api`.

After the first `comline build`, a `.comline/` directory appears next to
`config.idp`. It holds the content-addressable store (`objects/`) and the version
ref (`refs/heads/main`). It is build output — keep it out of version control (the
scaffold's `.gitignore` already does).

## Versioning model

Comline stores every build as an immutable commit in an **append-only chain**
under `.comline/`, git-inspired and content-addressed. There are no branches and
history is never rewritten, so any past version stays reproducible.

Each `build` compares the new schemas against the previous version and bumps the
package version automatically, by the largest change it finds:

| bump | when | examples |
|---|---|---|
| **major** | a breaking change | removed struct/enum/field/variant/function, changed field type, added a required field |
| **minor** | a new feature | added struct/enum/protocol/error, added a variant, added an optional field, new schema file |
| **patch** | a modification | field made optional, docstring-only change |

The first build is version `0.0.1`. A build with no schema changes keeps the
current version and does not add a commit. `comline diff` runs this same
comparison between any two stored versions on demand.

## Commands

Every command accepts the [global flags](#global-flags). Run `comline <cmd>
--help` for the authoritative synopsis.

### `comline new`

```
comline new <name> [--git]
```

Scaffold `<name>/` (see [Project layout](#project-layout)). Fails if the
directory already exists.

- `--git` — also run `git init` in the new project. If `git` is missing or fails,
  the project is still created and a warning is printed.

```bash
comline new my-api
comline new my-api --git
```

### `comline check`

```
comline check
```

Parse, resolve and validate every schema and the package config, reporting the
first error found. It writes **nothing** to `.comline/` and does **not** bump the
version — safe to run from editors, pre-commit hooks and CI lint steps.

```bash
comline check
comline --path ./services/users check
```

Exit `0` if valid, `1` if a schema or the config is invalid, `2` if the directory
is not a Comline project.

### `comline build`

```
comline build [--release] [--watch]
```

Compile and validate every schema, then freeze the result into `.comline/`,
bumping the version per the [versioning model](#versioning-model). Prints the
version change, a grouped changelog, and the bump that was applied.

- `--release` — reserved for future optimization work; currently a no-op (prints
  a note).
- `--watch` — rebuild on change; see [Watch mode](#watch-mode).

```bash
comline build
comline build --watch
```

### `comline generate`

```
comline generate [--target <lang>] [--out <dir>] [--layout <template>] [--mode <mode>] [--watch]
```

Validate the project, then write generated code for each target. Unlike `build`,
this **does not** freeze a version or write to `.comline/` — it stops after
validation.

**Targets** come from `[[generate.target]]` in `comline.toml`; if that file lists
none, from `code_generation.languages` in `config.idp`.

**Where the code goes** is set in `comline.toml`'s `[generate]` table:

```toml
[generate]
out    = "generated"                            # output root, relative to comline.toml
layout = "{{language}}/{{namespace}}.{{ext}}"   # path of each file under the root
mode   = "code"                                 # code | lib | dylib (only `code` today)

# optional, one per target you want to pin or override:
[[generate.target]]
language = "rust"
out      = "src/generated"
```

`comline.toml` is committed, consumer-owned, and never frozen. With no
`comline.toml` (or an all-commented one), the defaults above apply. Layout
variables: `{{language}}`, `{{namespace}}` (`/`-joined), `{{ext}}`,
`{{lang_version}}`, `{{spec_version}}`.

- `--target <lang>` — only this language (case-insensitive). Errors if nothing
  matches.
- `--out <dir>` / `--layout <template>` / `--mode <mode>` — override one target's
  `[generate]` values. When more than one target is configured they require
  `--target` (otherwise it is ambiguous which they apply to).
- `COMLINE_GENERATE_OUT` / `COMLINE_GENERATE_LAYOUT` / `COMLINE_GENERATE_MODE` —
  the same overrides from the environment, sitting just below the flags but
  applied to **every** target (handy in CI). A flag still wins for its target.
- `--watch` — regenerate on change; see [Watch mode](#watch-mode).

Currently `rust` is the only generator and `code` the only mode; other languages
or modes fail with a clear message.

```bash
comline generate
comline generate --target rust --out ./bindings
comline -p ./schemas generate --target rust
```

```
• Generating code (in schemas/)
  ⚙️  language: rust, version: 1.70.0
       main.ids → generated/rust/main.rs
       other.ids → generated/rust/other.rs
✓ Generated 2 file(s)
```

The working directory is named on the header; each line under a target maps a
source schema to the file written for it (`--plain` uses `->`).

### `comline diff`

```
comline diff <old> [<new>]
```

Compare two frozen versions from `.comline/` and print the breaking changes, new
features and modifications between them — the same report `build` shows, on
demand. `<new>` defaults to `HEAD`.

Each argument is one of:

- a **version string**, e.g. `0.2.0`
- a **commit hash** (full, or a prefix of 4+ characters)
- **`HEAD`** — the latest build

```bash
comline diff 0.1.0 0.2.0
comline diff 0.1.0            # 0.1.0 against HEAD
comline diff 0.0.1 HEAD
```

Fails with exit `2` if the project has never been built, and exit `1` if an
argument matches no stored version (the error lists the versions that exist).

### `comline clean`

```
comline clean [--dry-run]
```

Remove build artifacts: the `.comline/` store, and what `generate` would have
written — the whole output root when it is a dedicated directory (e.g.
`generated/`), otherwise just the individual generated files. The next `build`
starts a fresh version history at `0.0.1`.

- `--dry-run` — list what would be removed without deleting anything.

Generated-file cleanup needs the project to compile; if it doesn't, `.comline/`
is still removed and a note is printed.

```bash
comline clean --dry-run
comline clean
```

### `comline completions`

```
comline completions <shell>
```

Print a completion script for `<shell>` to **stdout**. Supported: `bash`, `zsh`,
`fish`, `powershell`, `elvish`.

```bash
comline completions bash > /etc/bash_completion.d/comline
comline completions fish > ~/.config/fish/completions/comline.fish
comline completions zsh  > ~/.zfunc/_comline
```

## Global flags

| flag | effect |
|---|---|
| `-p`, `--path <DIR>` | Run against `<DIR>` instead of the current directory. Headers echo it back (`Building project (in <DIR>/)`). |
| `-v`, `-vv`, `-vvv` | Raise log verbosity. Default shows only warnings and errors from `comline-core`; `-v` adds info, `-vv` debug, `-vvv` trace. |
| `-q`, `--quiet` | Silence all progress output; only errors are printed. Conflicts with `-v`. |
| `--plain` | Plain output: no color, no leading symbols (`•` / `✓`), no emoji in the changelog, no spinner. For log files and CI. |

`RUST_LOG` overrides the verbosity flags entirely if set (standard
`tracing_subscriber` syntax, e.g. `RUST_LOG=comline_core=debug`).

## Output and exit codes

- **stderr** carries all human-facing output: progress steps, the changelog,
  warnings, errors, and `-v` diagnostics.
- **stdout** is reserved for machine-readable payloads. Today only
  `comline completions` writes there, so `comline completions fish | source`
  and similar pipelines are clean.
- Color is applied through `anstream`: it is stripped automatically when stderr
  is not a terminal or when `NO_COLOR` is set. `--plain` also drops the leading
  symbols and changelog emoji, not just color.
- The progress spinner is shown only on an interactive terminal, and is
  suppressed under `-v` (so it doesn't fight the log output), `--quiet` and
  `--plain`.

Exit codes are stable for scripting and CI:

| code | meaning |
|---|---|
| `0` | success |
| `1` | the command ran but failed — invalid schema, unresolved `diff` argument, a generator or filesystem error |
| `2` | a precondition was not met — the directory is not a Comline project, or nothing has been built yet. `clap` also exits `2` for usage errors. |

```bash
if ! comline check --quiet; then
  echo "schemas are invalid" >&2
  exit 1
fi
```

## Watch mode

`comline build --watch` and `comline generate --watch` run the action once, then
re-run it whenever a file under `src/` changes or `config.idp` / `comline.toml`
is modified
(300 ms debounce). A failing run is reported but does **not** stop the loop —
fix the error and save again. Press Ctrl-C to exit.

```bash
comline generate --watch --target rust
```

## Shell completions and man pages

Two paths, for two audiences:

- **Users**: `comline completions <shell>` (above) prints a script on demand.
- **Packagers**: every build also writes, into the crate's `OUT_DIR`:
  - `man/comline.1` and `man/comline-<subcommand>.1`
  - `completions/comline.{bash,zsh,fish,elv}` and `_comline` for PowerShell

  After `cargo build`, find them under
  `target/<profile>/build/comline-*/out/{man,completions}/`.

## Editor and pre-commit integration

Use `comline check` — it validates without writing to `.comline/` or bumping the
version, so it is safe to run on every save or in a hook.

```sh
# .git/hooks/pre-commit
#!/bin/sh
exec comline check --quiet
```
