//! `comline.toml`, the CLI's side of it.
//!
//! The file's types, parsing and resolution live in
//! [`comline_codegen::config`], shared with the language server and the
//! JSON Schema editors validate it with. What stays here needs the
//! compiler: rendering a target's `layout` into a path, with core's template
//! engine.

use std::path::PathBuf;

use comline_codegen::config::ResolvedTarget;
use comline_core::utils::templating::recurse_render;
use miette::{miette, Result};
use serde::Serialize;

/// Variables a `layout` template can reference.
#[derive(Serialize)]
struct LayoutVars<'a> {
    language: &'a str,
    namespace: &'a str,
    ext: &'a str,
    lang_version: &'a str,
    spec_version: &'a str,
    package_version: &'a str,
}

/// Where a resolved target writes each file.
pub trait Destination {
    /// Render `layout` for one schema namespace into an absolute path under
    /// `out`. `ext` is the generator's file extension (e.g. `rs`);
    /// `package_version` is the version being generated (empty for an unbuilt
    /// working tree).
    fn dest_for(
        &self,
        namespace: &str,
        ext: &str,
        spec_version: &str,
        package_version: &str,
    ) -> Result<PathBuf>;
}

impl Destination for ResolvedTarget {
    fn dest_for(
        &self,
        namespace: &str,
        ext: &str,
        spec_version: &str,
        package_version: &str,
    ) -> Result<PathBuf> {
        let vars = LayoutVars {
            language: &self.language,
            namespace,
            ext,
            lang_version: &self.lang_version,
            spec_version,
            package_version,
        };
        let rel = recurse_render(&self.layout, &vars)
            .map_err(|e| miette!("bad `layout` template `{}`: {e}", self.layout))?;
        Ok(self.out.join(rel))
    }
}
