//! The official Rust generator as an external one: what comline-rust's
//! `comline-gen-rust` binary is.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut registry = comline_codegen::Registry::new();
    comline_codegen_rust::register(&mut registry);
    comline_codegen::protocol::serve_stdio(&registry)
}
