//! End-to-end tests for `pb docs`: invoke the real binary and assert it writes
//! byte-identical embedded reference text to stdout and nothing to stderr, so an
//! LLM (or pipe) capturing fd 1 gets the exact spec with no contamination.

use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn isolated_command(args: &[&str]) -> std::process::Output {
    // Isolate config/state so the command never touches the real environment.
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let tmp = std::env::temp_dir().join(format!(
        "pb-docs-cli-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&tmp).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_peanutbutter"))
        .args(args)
        .env("PB_CONFIG_FILE", tmp.join("config.toml"))
        .env("XDG_CONFIG_HOME", tmp.join("config"))
        .env("XDG_STATE_HOME", tmp.join("state"))
        .output()
        .expect("run peanutbutter binary");
    let _ = std::fs::remove_dir_all(&tmp);
    output
}

fn canonical(rel: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)).unwrap()
}

#[test]
fn docs_syntax_prints_canonical_file_to_clean_stdout() {
    let output = isolated_command(&["docs", "syntax"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, canonical("docs/SNIPPET_SYNTAX.md"));
    assert!(output.stderr.is_empty(), "stderr: {:?}", output.stderr);
}

#[test]
fn docs_config_prints_canonical_file_to_clean_stdout() {
    let output = isolated_command(&["docs", "config"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, canonical("examples/config.toml"));
    assert!(output.stderr.is_empty(), "stderr: {:?}", output.stderr);
}

#[test]
fn docs_without_topic_lists_topics() {
    let output = isolated_command(&["docs"]);
    assert!(output.status.success());
    assert_eq!(output.stdout, b"syntax\nconfig\n");
    assert!(output.stderr.is_empty(), "stderr: {:?}", output.stderr);
}
