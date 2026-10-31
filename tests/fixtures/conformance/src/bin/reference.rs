//! `comline-reference`, built from the pinned runtime rev.

fn main() -> std::io::Result<()> {
    comline_reference::serve(std::io::stdin().lock(), std::io::stdout().lock())
}
