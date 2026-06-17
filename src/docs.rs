//! Embedded reference documentation served by `pb docs <topic>`.
//!
//! The canonical sources live at `docs/SNIPPET_SYNTAX.md` and
//! `examples/config.toml`. `build.rs` copies them into `OUT_DIR/assets/` at
//! compile time and the consts below embed those copies, so a released binary
//! (or `cargo install`) can print the exact spec text offline — handy for an LLM
//! that would otherwise fetch the file from GitHub.
//!
//! Output is intentionally raw: the embedded bytes are written verbatim with no
//! color or status text, even on a TTY, so piping into another tool stays clean.
//!
//! Note: this is unrelated to the [`crate::syntax`] module, which parses snippet
//! command templates. [`Topic::Syntax`] is pure reference text.

use clap::ValueEnum;
use std::io::{self, Write};

/// The snippet syntax reference (`docs/SNIPPET_SYNTAX.md`), embedded from the
/// `build.rs`-generated copy in `OUT_DIR`.
pub const SNIPPET_SYNTAX: &str =
    include_str!(concat!(env!("OUT_DIR"), "/assets/snippet_syntax.md"));

/// The annotated example config (`examples/config.toml`), embedded from the
/// `build.rs`-generated copy in `OUT_DIR`.
pub const CONFIG_EXAMPLE: &str = include_str!(concat!(env!("OUT_DIR"), "/assets/config.toml"));

/// A reference document that `pb docs` can print to stdout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum Topic {
    /// The snippet syntax reference (`docs/SNIPPET_SYNTAX.md`).
    Syntax,
    /// The annotated example config (`examples/config.toml`).
    Config,
}

/// Write the requested reference [`Topic`] to `writer` as raw bytes, or — when no
/// topic is given — the list of available topics so callers can enumerate them.
///
/// Output is byte-identical to the canonical source file; nothing else (color,
/// headers, trailing status) is written.
pub fn run<W: Write>(topic: Option<Topic>, writer: &mut W) -> io::Result<()> {
    match topic {
        Some(Topic::Syntax) => writer.write_all(SNIPPET_SYNTAX.as_bytes()),
        Some(Topic::Config) => writer.write_all(CONFIG_EXAMPLE.as_bytes()),
        None => writer.write_all(b"syntax\nconfig\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The drift guards compare the embed against the canonical file read
    // *directly from the source tree*. `include_str!` here is resolved by the
    // compiler relative to this file (`src/`), so it reads `docs/` and
    // `examples/` — not the `OUT_DIR` copy. Comparing two `OUT_DIR` reads would
    // compare the embed against itself and catch no drift.
    #[test]
    fn syntax_embed_matches_canonical_source() {
        assert_eq!(SNIPPET_SYNTAX, include_str!("../docs/SNIPPET_SYNTAX.md"));
    }

    #[test]
    fn config_embed_matches_canonical_source() {
        assert_eq!(CONFIG_EXAMPLE, include_str!("../examples/config.toml"));
    }

    #[test]
    fn embeds_are_non_empty() {
        // A zero-byte OUT_DIR copy from a build.rs bug should fail loudly here.
        assert!(SNIPPET_SYNTAX.len() > 100);
        assert!(CONFIG_EXAMPLE.len() > 100);
    }

    #[test]
    fn embeds_contain_known_content_markers() {
        // Catches a truncated embed even if non-empty.
        assert!(SNIPPET_SYNTAX.contains("# Snippet Syntax"));
        assert!(CONFIG_EXAMPLE.contains("[search.fuzzy]"));
    }

    #[test]
    fn run_writes_syntax_verbatim() {
        let mut out = Vec::new();
        run(Some(Topic::Syntax), &mut out).unwrap();
        assert_eq!(out, SNIPPET_SYNTAX.as_bytes());
    }

    #[test]
    fn run_writes_config_verbatim() {
        let mut out = Vec::new();
        run(Some(Topic::Config), &mut out).unwrap();
        assert_eq!(out, CONFIG_EXAMPLE.as_bytes());
    }

    #[test]
    fn run_without_topic_lists_topics() {
        let mut out = Vec::new();
        run(None, &mut out).unwrap();
        assert_eq!(out, b"syntax\nconfig\n");
    }
}
