//! Integration tests for the `comline` binary, one module per subcommand area.
//!
//! Shared helpers live in [`util`]. Cargo compiles this directory as a single
//! test target (`tests/cli/main.rs`), so `cargo test` runs the whole suite and
//! `cargo test <module>::` scopes to one area (e.g. `cargo test generate::`).

mod util;

mod accept;
mod add;
mod build;
mod check;
mod clean;
mod conformance;
mod diff;
mod end_to_end;
mod external;
mod generate;
mod global;
mod new;
mod release;
mod reset;
