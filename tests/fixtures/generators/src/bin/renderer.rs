//! The official Markdown renderer as an external one: what generation's
//! `comline-doc-markdown` binary is.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut registry = comline_codegen::Registry::new();
    comline_doc_markdown::register(&mut registry);
    comline_codegen::protocol::serve_stdio(&registry)
}
