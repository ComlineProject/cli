//! Command-line surface for `comline`.
//!
//! This module is intentionally self-contained (only `clap`, `clap_complete` and
//! `std`) so that `build.rs` can `#[path]`-include it to generate man pages and
//! shell completions from the exact same definitions the binary uses.

use std::path::PathBuf;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

#[derive(Parser, Debug)]
#[command(
    name = "comline",
    version,
    about = "Build, validate, diff and generate code from Comline schemas",
    long_about = None,
)]
pub struct Cli {
    /// Run against this directory instead of the current one
    #[arg(short, long, global = true, value_name = "DIR")]
    pub path: Option<PathBuf>,

    /// Increase log verbosity (-v info, -vv debug, -vvv trace); surfaces comline-core diagnostics
    #[arg(
        short,
        long,
        global = true,
        action = clap::ArgAction::Count,
        conflicts_with = "quiet",
    )]
    pub verbose: u8,

    /// Silence progress output; only errors are printed
    #[arg(short, long, global = true)]
    pub quiet: bool,

    /// Plain output: no color, no symbols, no progress animation (for logs / CI)
    #[arg(long, global = true)]
    pub plain: bool,

    #[command(subcommand)]
    pub command: Commands,
}

/// How far `comline release --raise` goes.
#[derive(Debug, Clone, Copy, clap::ValueEnum)]
pub enum ReleaseLevel {
    Patch,
    Minor,
    Major,
}

#[derive(Subcommand, Debug)]
pub enum Commands {
    /// Compile the project and record a new build
    #[command(long_about = "\
Compile and validate every schema, then freeze the result into the project's \
content-addressable store (`.comline/`). A build is recorded under a \
pre-release version: the release it leads to and a count (`0.3.0-dev.2`). \
The release follows the largest change since the last release; \
`comline release` makes it.

Examples:
  comline build
  comline build --watch")]
    Build {
        /// Rebuild automatically when a schema or `config.idp` changes
        #[arg(short, long)]
        watch: bool,
    },

    /// Validate the project without freezing a version or writing artifacts
    #[command(long_about = "\
Parse, resolve and validate every schema and the project config, reporting the \
first error found. Unlike `build`, this never writes to `.comline/` and never \
bumps the version, so it is safe to run in editors and pre-commit hooks.")]
    Check,

    /// Show what each protocol speaks on the wire and which guarantees it keeps
    #[command(long_about = "\
For each protocol, the format, framing and handshake it resolves to, their \
parameters, and where each came from: the protocol's own `@framing` or \
`@handshake`, the package's `wire` or `handshake` in `config.idp`, or core's \
default. Then every guarantee (schema agreement, encoding agreement, encoding \
conformance, version honesty), held or given up, with the cause, the \
`@unverified` reason, and whether it has been accepted. A dependency's \
protocol is accepted by this package's `accept_unverified`; who accepts this \
package's own protocols isn't known here.

Like `check`, this never writes to `.comline/`. `--json` prints the same as \
JSON.

Examples:
  comline inspect
  comline inspect --json")]
    Inspect {
        /// Print the report as JSON
        #[arg(long)]
        json: bool,
    },

    /// Write the package's documentation: Markdown pages, or the model as JSON
    #[command(long_about = "\
Write the package's documentation: an index page, one page per module (its \
`//!` header, every unit with its `///` docs, fields, arguments and each \
protocol's wire summary), a changelog page with `--changelog-from`, and an \
mkdocs/zensical `nav.yml`. Read-only: nothing is written to `.comline/`.

The working tree is documented unless `--version` names a built version (a \
version string, a commit hash or `HEAD`), which is documented from its frozen \
IR, `//!` headers included. A built version keeps no source, so its protocols \
have no wire summary.

`--format html` writes a self-contained static site instead: `index.html`, \
one page per module, `changelog.html`, a stylesheet and a search script, all \
linked relatively, so `index.html` opens straight from disk. `--format json` \
writes the documentation model itself (`model.json`), for other tools. `comline.toml`'s `[doc]` table sets the defaults: `out` \
(`docs/api`), `format` (`markdown`), `renderer` (an external renderer, run \
like an external generator), `links` (dependency docs URLs) and \
`changelog_from`.

Examples:
  comline doc
  comline doc --out site/api --changelog-from 0.1.0
  comline doc --version 0.2.0
  comline doc --format html --out site
  comline doc --format json")]
    Doc {
        /// Output directory; default from `[doc] out`, else `docs/api`
        #[arg(long, value_name = "DIR")]
        out: Option<std::path::PathBuf>,

        /// Document this built version instead of the working tree
        #[arg(long = "version", value_name = "VERSION")]
        doc_version: Option<String>,

        /// Add a changelog from this built version to the documented one
        #[arg(long, value_name = "VERSION")]
        changelog_from: Option<String>,

        /// `markdown` (pages), `html` (a static site) or `json` (the model); default from `[doc] format`
        #[arg(long, value_name = "FORMAT")]
        format: Option<String>,
    },

    /// Validate the project and write generated code for each configured target
    #[command(long_about = "\
Validate the project (no version bump, no `.comline/` write), then write \
generated code for each target.

Every language `config.idp` declares under `code_generation.languages` is \
generated. A `[[generate.target]]` block in `comline.toml` configures one of \
them (where it lands, in what mode); a language with no block takes the \
`[generate]` defaults. Narrow the set with `[generate] only` for good, or \
`--target` for one run. Output location and layout live in `comline.toml`'s \
`[generate]` table. `--out` / `--layout` / \
`--mode` override one target and need `--target` when several are configured. \
Defaults: out `generated`, layout `{{language}}/{{namespace}}.{{ext}}`, mode \
`code` (`lib` emits a buildable package instead; `dylib` is not built yet).

Each target's files are drawn as a tree. `--list` prints them one per line instead, with the schema each came from. `--dry-run` writes nothing, and reports the same way.

Examples:
  comline generate
  comline generate --target rust --out ./bindings
  comline generate --watch")]
    Generate {
        /// Only generate for this language (e.g. `rust`); default: every configured target
        #[arg(short, long, value_name = "LANG")]
        target: Option<String>,

        /// Output root for the selected target (overrides `comline.toml`)
        #[arg(short, long, value_name = "DIR")]
        out: Option<PathBuf>,

        /// Layout template for the selected target, e.g. `{{language}}/{{namespace}}.{{ext}}`
        #[arg(short, long, value_name = "TEMPLATE")]
        layout: Option<String>,

        /// Emit form for the selected target: `code` or `lib` (`dylib` is not built yet)
        #[arg(short, long, value_name = "MODE")]
        mode: Option<String>,

        /// Reference `comline_std` as its own package instead of inlining it
        /// (Rust only; the default in `lib` mode, off in `code` mode)
        #[arg(long, conflicts_with = "no_external_std")]
        external_std: bool,

        /// Inline `std` into the generated output instead of referencing it
        #[arg(long)]
        no_external_std: bool,

        /// Run `@validators` checks while decoding: `off`, `collect` (fail
        /// with every failure) or `first` (fail with the first)
        #[arg(long, value_name = "WHEN")]
        validate_on_decode: Option<String>,

        /// Print what would be written, without writing anything
        #[arg(long)]
        dry_run: bool,

        /// List each file on its own line instead of drawing a tree
        #[arg(long)]
        list: bool,

        /// Regenerate automatically when a schema or `config.idp` changes
        #[arg(short, long)]
        watch: bool,
    },

    /// Show the schema changes between two built versions
    #[command(long_about = "\
Compare two frozen versions from the project's `.comline/` store and print the \
breaking changes, new features and modifications between them — the same \
report `build` shows, on demand.

Each argument is a version string (`0.2.0`), a commit hash, or `HEAD`.

Examples:
  comline diff 0.1.0 0.2.0
  comline diff 0.1.0            # compares 0.1.0 against HEAD")]
    Diff {
        /// Base version: a version string, a commit hash, or `HEAD`
        #[arg(value_name = "OLD")]
        old: String,

        /// Target version: a version string, a commit hash, or `HEAD`
        #[arg(value_name = "NEW", default_value = "HEAD")]
        new: String,
    },

    /// Remove generated code
    #[command(long_about = "\
Remove the files `comline generate` writes — the whole output root when it is a \
dedicated directory (e.g. `generated/`), otherwise the individual generated \
files. Does not touch `.comline/`; use `comline reset` to discard the version \
history.")]
    Clean {
        /// List what would be removed without deleting anything
        #[arg(long)]
        dry_run: bool,
    },

    /// Release the builds since the last release as a version
    #[command(long_about = "\
Builds record history under pre-release versions (`0.3.0-dev.2`: the release \
they lead to, and a count). A release is the version consumers see. This builds \
first, so what's released is what's in `src/` now, then creates the version from \
the highest bump the changes since the last release call for: a breaking change \
is a major (before 1.0, the minor), a new feature a minor, anything else a patch. \
The first release is 0.1.0.

`--raise` releases a higher bump than the changes show (a behaviour change the \
schema can't express), with a `--reason` recorded with the release. It never \
lowers the bump.

`comline release 1.0` makes a 0.x package stable: it says what 1.0 commits to \
and what the package exposes, then releases 1.0.0.

Raising to major and releasing 1.0 ask for confirmation on a terminal; \
otherwise pass `--yes`.

Examples:
  comline release
  comline release --raise minor --reason \"`ping` now retries on timeout\"
  comline release 1.0")]
    Release {
        /// `1.0`, to make a 0.x package stable; otherwise omitted
        version: Option<String>,

        /// Release a higher bump than the changes call for
        #[arg(long, value_enum, value_name = "LEVEL")]
        raise: Option<ReleaseLevel>,

        /// Why: required with `--raise`, recorded with the release
        #[arg(long, value_name = "TEXT")]
        reason: Option<String>,

        /// Don't ask for confirmation
        #[arg(long, short)]
        yes: bool,
    },

    /// Discard the version history (delete `.comline/`)
    #[command(long_about = "\
Delete the project's `.comline/` content-addressable store — every frozen \
version and the commit chain. Generated code is removed too. This cannot be \
undone and nothing else holds the history, so the next `build` starts over at \
0.0.1.

Requires confirmation: on a terminal you are asked to type `reset`; otherwise \
pass `--force`.")]
    Reset {
        /// Skip the confirmation prompt
        #[arg(long)]
        force: bool,

        /// List what would be removed without deleting anything
        #[arg(long)]
        dry_run: bool,
    },

    /// Create a new Comline project in a new directory
    New {
        /// Project name; also the directory that gets created
        name: String,

        /// Also run `git init` inside the new project
        #[arg(long)]
        git: bool,
    },

    /// Add a dependency to `config.idp`, resolved and pinned
    #[command(long_about = "\
Declare another Comline package as a dependency. It is resolved first, the way \
`check` resolves it (a git pin is fetched into `.comline/deps-cache/`, then \
compiled), so an entry that wouldn't resolve is never written. Its entry then \
goes into `config.idp`'s `dependencies` block, with `hash` pinned to the content \
just compiled: if the dependency changes later, the build fails until the pin is \
updated. The rest of `config.idp` is left as written.

Its schemas are imported under NAME: `use NAME::models::Thing`.

If it has `@unverified` protocols, what each gives up is shown and, on a \
terminal, you are asked to accept it (see `comline accept`); otherwise pass \
`--accept-unverified`. Nothing is written unless it's accepted.

Examples:
  comline add shared_types ../shared-types
  comline add net --git https://github.com/acme/net --commit 4f2c9e1 --version 1.2.0")]
    Add {
        /// Name to import it under (`use <NAME>::...`)
        name: String,

        /// A package on disk, relative to this package's directory
        #[arg(
            value_name = "DIR",
            required_unless_present = "git",
            conflicts_with = "git"
        )]
        dir: Option<PathBuf>,

        /// Fetch it from this git repository instead
        #[arg(long, value_name = "URI", requires_all = ["commit", "version"])]
        git: Option<String>,

        /// Commit to pin (with `--git`)
        #[arg(long, value_name = "SHA", requires = "git")]
        commit: Option<String>,

        /// Version that commit stands for (with `--git`)
        #[arg(long, value_name = "VERSION", requires = "git")]
        version: Option<String>,

        /// Don't pin its content hash (say, a path dependency still being worked on)
        #[arg(long)]
        no_hash: bool,

        /// Accept what its `@unverified` protocols give up without being asked
        #[arg(long)]
        accept_unverified: bool,
    },

    /// Accept what a dependency's `@unverified` protocols give up
    #[command(long_about = "\
A protocol marked `@unverified` gives up guarantees Comline otherwise gives its \
peers (a `none` handshake gives up schema and encoding agreement), and a \
package depending on it has to accept that in its own `config.idp`, for exactly \
what's given up. This shows what each protocol gives up and why, then writes \
`accept_unverified` into the dependency's entry. The rest of `config.idp` is \
left as written.

Run it again when a dependency's update gives up more: the build stops until \
the new set is accepted.

Requires confirmation: on a terminal you are asked; otherwise pass `--yes`.

Examples:
  comline accept billing            every unverified protocol in `billing`
  comline accept billing::Billing   just `Billing`")]
    Accept {
        /// The dependency, or one protocol in it: `NAME` or `NAME::PROTOCOL`
        target: String,

        /// Accept without being asked
        #[arg(long, short)]
        yes: bool,
    },

    /// Run an encoding's implementation against its vectors
    #[command(long_about = "\
Run an implementation of the package's encodings against the vectors they \
ship, the way the official runtimes are proven against the official corpus. \
Each encoding declared under `encodings` in config.idp ships its vectors \
beside its profile: `profiles/lines.json` has `profiles/lines.vectors.json`, \
in the conformance corpus's case format.

The implementation is a process speaking the conformance ops protocol (as \
`comline-reference` does): request lines on stdin, a reply line each on \
stdout, exit when stdin ends. It runs once, with a cleared environment, a \
time limit and an output cap. The report says which profile facts are \
proven and which cases failed, with byte diffs; the exit code is 1 on any \
failure.

`--corpus` runs a corpus laid out like ComlineProject/runtime's \
`runtime/conformance` (`profiles/`, `vectors/`) instead, for the official \
encodings.

Examples:
  comline conformance --impl ./target/release/acme-lines-conformance
  comline conformance --impl \"node conformance.js\" --encoding com.acme.lines@1
  comline conformance --impl comline-reference --corpus runtime/conformance --encoding packed@1")]
    Conformance {
        /// The implementation's command line: a program and its arguments, split on whitespace
        #[arg(long = "impl", value_name = "COMMAND")]
        implementation: String,

        /// Only this encoding (repeatable); default: every encoding the package declares
        #[arg(long, value_name = "NAME")]
        encoding: Vec<String>,

        /// Run a runtime-style corpus directory (profiles/, vectors/) instead of the package's encodings
        #[arg(long, value_name = "DIR")]
        corpus: Option<PathBuf>,

        /// Seconds the implementation may take for every case together [default: 60]
        #[arg(long, value_name = "SECS")]
        timeout: Option<u64>,
    },

    /// Print a shell completion script to stdout
    #[command(long_about = "\
Write a completion script for the given shell to stdout.

Examples:
  comline completions bash > /etc/bash_completion.d/comline
  comline completions fish > ~/.config/fish/completions/comline.fish")]
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: Shell,
    },

    /// Print every language#version this build can generate, to stdout
    #[command(long_about = "\
Print one line per generator this binary of `comline` was compiled with: \
its `language#version` token, the syntax `config.idp`'s \
`code_generation.languages` expects, then in parentheses the versions it \
handles. The version in the token is the lowest; a declared version is a \
minimum, so any later one it handles works too. Ignores `--path`: this \
reflects the build, not any particular project.

With `--features`, also what each generator implements, feature by feature: \
supported, partial, ignored (generated without it) or unsupported (refused), \
each claim proven by the generators' conformance suite. `--json` prints the \
same as JSON.

Examples:
  comline targets
  comline targets | grep '^rust#'
  comline targets | cut -d' ' -f1
  comline targets --features
  comline targets --features --json")]
    Targets {
        /// Also list what each generator supports, feature by feature
        #[arg(long)]
        features: bool,
        /// With `--features`: print it as JSON
        #[arg(long, requires = "features")]
        json: bool,
    },
}
