# Releasing & cross-repo workflow

`comline` (this repo) is the CLI. It depends on **`comline-core`**
(`ComlineProject/core`), which is currently consumed as a **git dependency**
pinned to an exact commit in [`Cargo.toml`](Cargo.toml):

```toml
comline-core = { git = "https://github.com/ComlineProject/core", rev = "<sha>" }
```

Branching model is GitHub Flow: short-lived feature branches → PR → CI → `main`.
Releases are **tags** cut from `main`, not long-lived branches. There is no
`develop`/staging branch.

## Local development loop

Per-developer config lives in a **gitignored** `.cargo/config.toml`:

- `[build] target-dir` — required here (the checkout is on a `noexec` mount, so
  cargo can't run build scripts from `./target`).
- An optional `[patch."https://github.com/ComlineProject/core"]` pointing at a
  local `core` checkout. **Uncomment it** to test `comline-core` changes before
  they are pushed; **re-comment it** to build against the pinned `rev` (what CI
  and releases use). The committed `Cargo.toml` never carries a patch.

`Cargo.lock` **is committed** (this is a binary): CI and `cargo install` build
the exact versions that ship.

## Landing a `comline-core` change the CLI needs

1. **core**: feature branch → PR against `ComlineProject/core` → CI green → merge
   to `master`. Note the merge commit SHA.
2. **cli**: bump the `rev` in `Cargo.toml` to that SHA, then
   `cargo update -p comline-core`.
3. **cli**: `cargo test`, `cargo clippy --all-targets -- -D warnings`,
   `cargo fmt --check`.
4. **cli**: commit `Cargo.toml` **and** `Cargo.lock` together → PR → merge.

(While iterating, step 2–3 happen against the local `[patch]` first; the `rev`
bump is the last step once the core PR has merged.)

## Cutting a CLI release (once `comline-core` is on crates.io)

1. **core**: bump `version` in `core/core/Cargo.toml`, `cargo publish -p comline-core`,
   tag `core-vX.Y.Z`.
2. **cli**: change the dependency from git to a version —
   `comline-core = "X.Y"` — and drop the git NOTE comment. `cargo update`,
   re-run the full check suite.
3. **cli**: bump `version` in `Cargo.toml`, commit `Cargo.toml` + `Cargo.lock`.
4. **cli**: `cargo publish` (a clean `cargo package` first to sanity-check the
   file list), then tag `vX.Y.Z` on the release commit and push the tag.

Until then, "release" just means `main` is green and
`cargo install --path .` works.

## Pre-publish checklist (crates.io)

- [ ] `comline-core` is published to crates.io (blocks everything below).
- [ ] `Cargo.toml`: dependency is a version, not a git URL.
- [ ] `Cargo.toml`: `version`, `description`, `repository`, `license-file`,
      `keywords`, `categories`, `readme` all correct.
- [ ] `cargo package --list` contains no stray files (fixtures, local configs).
- [ ] `cargo test` / `clippy -D warnings` / `fmt --check` green on `main`.
- [ ] Tag `vX.Y.Z` pushed.
