# comline

The command-line interface for [Comline](https://github.com/ComlineProject) —
build, validate, diff and generate code from Comline schemas.

> Comline is in early development. The schema language and CLI surface may still
> change between versions.

## Install

`comline` is not on crates.io yet (it depends on `comline-core` via git until
that crate is published). Build it from this repository:

```bash
git clone https://github.com/ComlineProject/cli
cargo install --path cli
```

## Quick start

```bash
comline new my-api          # scaffold a project
cd my-api
comline build               # compile + freeze version 0.0.1
comline generate            # write generated code for each configured target
```

## Commands

| Command | What it does |
|---|---|
| `comline new <name> [--git]` | Scaffold `<name>/` with `config.idp`, `src/main.ids` and a `.gitignore`. `--git` also runs `git init`. |
| `comline check` | Parse, resolve and validate every schema. No `.comline/` writes, no version bump — safe for editors, hooks and CI. |
| `comline build [--release] [--watch]` | Compile, validate, and freeze a new immutable version into `.comline/`. Prints the changelog and the version bump. |
| `comline generate [--target <lang>] [--watch]` | Build, then run each configured code generator (or just `<lang>`). |
| `comline diff <old> <new>` | Show the schema changes between two built versions. Each argument is a version (`0.2.0`), a commit hash, or `HEAD` (the default for `<new>`). |
| `comline clean [--dry-run]` | Remove `.comline/` and generated files. |
| `comline completions <shell>` | Print a shell completion script to stdout. |

Global flags: `--path <dir>` to run outside the current directory, `-v`/`-vv` for
more log detail, `-q`/`--quiet` to silence everything but errors.

## Versioning model

Comline stores every build as an immutable commit in an append-only chain under
`.comline/` (content-addressable, git-inspired). Each `build` compares the new
schemas against the previous version and bumps automatically:

- **major** — a breaking change (removed/retyped field, removed variant, …)
- **minor** — a new feature (added struct/enum/variant/field, …)
- **patch** — a modification (field made optional, docstring change, …)

`comline diff` runs that same comparison between any two stored versions on
demand.

## Exit codes

| code | meaning |
|---|---|
| `0` | success |
| `1` | the command ran but failed |
| `2` | a precondition was not met (not a Comline project, nothing built yet) or a usage error |

## Development

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Man pages and completion scripts for all shells are generated into `OUT_DIR`
(`target/.../build/comline-*/out/{man,completions}/`) on every build.
