//! `--help` and `--version` must work even when the user's config is unparseable
//! — that is exactly when someone runs `pb --help` to recover. The binary loads
//! config up front but defers the failure until after clap parsing, so help and
//! version render before any config requirement applies.

use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

fn run_with_broken_config(args: &[&str]) -> std::process::Output {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    let tmp = std::env::temp_dir().join(format!(
        "pb-help-nocfg-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&tmp).unwrap();
    let cfg = tmp.join("config.toml");
    std::fs::write(&cfg, "this is = = not valid toml [[[\n").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_peanutbutter"))
        .args(args)
        .env("PB_CONFIG_FILE", &cfg)
        .env("XDG_CONFIG_HOME", tmp.join("config"))
        .env("XDG_STATE_HOME", tmp.join("state"))
        .output()
        .expect("run peanutbutter binary");
    let _ = std::fs::remove_dir_all(&tmp);
    output
}

#[test]
fn help_flag_works_with_unparseable_config() {
    let output = run_with_broken_config(&["--help"]);
    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("terminal snippet manager"));
    // The dynamic snippet-root section is omitted (config failed to load), but
    // the base help — including the new docs command — still renders.
    assert!(stdout.contains("docs"));
    assert!(output.stderr.is_empty(), "stderr: {:?}", output.stderr);
}

#[test]
fn version_flag_works_with_unparseable_config() {
    let output = run_with_broken_config(&["--version"]);
    assert!(output.status.success(), "stderr: {:?}", output.stderr);
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .starts_with("peanutbutter ")
    );
    assert!(output.stderr.is_empty(), "stderr: {:?}", output.stderr);
}

#[test]
fn config_dependent_command_still_fails_on_bad_config() {
    // The deferred config error must still fire for commands that need config.
    let output = run_with_broken_config(&["lint"]);
    assert_eq!(output.status.code(), Some(2));
}
