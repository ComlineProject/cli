//! The official HTML renderer as an external one: what generation's
//! `comline-doc-html` binary is.

use std::process::ExitCode;

fn main() -> ExitCode {
    let mut registry = comline_codegen::Registry::new();
    comline_doc_html::register(&mut registry);
    comline_codegen::protocol::serve_stdio(&registry)
}
