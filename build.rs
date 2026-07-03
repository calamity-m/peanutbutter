//! Build script that copies canonical reference and starter files into
//! `OUT_DIR` so the binary can embed them with `include_str!`.
//!
//! The canonical sources stay where users and the README expect them
//! (`docs/SNIPPET_SYNTAX.md`, `examples/config.toml`, and
//! `examples/starter_snippets.md`); copying them into `OUT_DIR/assets/` keeps the
//! embed an explicit, generated artifact. These files are git-tracked and
//! packaged today, so this also works for a clean `cargo install` from crates.io.
//! Source-vs-embed tests guard against the embeds drifting from the canonical
//! files.

use std::env;
use std::fs;
use std::path::Path;

fn main() {
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR is set by cargo for build scripts");
    let assets = Path::new(&out_dir).join("assets");
    // `fs::copy` will not create the parent directory, and it does not pre-exist.
    fs::create_dir_all(&assets).expect("create OUT_DIR/assets");

    fs::copy("docs/SNIPPET_SYNTAX.md", assets.join("snippet_syntax.md"))
        .expect("copy docs/SNIPPET_SYNTAX.md into OUT_DIR/assets");
    fs::copy("examples/config.toml", assets.join("config.toml"))
        .expect("copy examples/config.toml into OUT_DIR/assets");
    fs::copy(
        "examples/starter_snippets.md",
        assets.join("starter_snippets.md"),
    )
    .expect("copy examples/starter_snippets.md into OUT_DIR/assets");

    // Re-run only when an input changes, otherwise incremental builds keep the
    // stale embed. Cover both sources and this script itself.
    println!("cargo:rerun-if-changed=docs/SNIPPET_SYNTAX.md");
    println!("cargo:rerun-if-changed=examples/config.toml");
    println!("cargo:rerun-if-changed=examples/starter_snippets.md");
    println!("cargo:rerun-if-changed=build.rs");
}
