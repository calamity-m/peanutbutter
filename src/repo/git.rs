//! Git subprocess helpers for `pb repo`.
//!
//! All commands capture stdout/stderr; nothing is inherited, so the TUI (and
//! the shell-integration stdout contract) can never be polluted by git output.

use std::io;
use std::path::Path;
use std::process::{Command, Output};

/// Repository actions runnable from the `pb repo` TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitOperation {
    /// Commit all changes, pull with rebase, then push.
    Sync,
    /// Commit all changes, then push.
    Push,
    /// Pull with rebase.
    Pull,
}

impl GitOperation {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Push => "push",
            Self::Pull => "pull",
        }
    }
}

/// Lightweight status shown per repository row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitSummary {
    /// Current branch name, or `HEAD` when detached.
    pub branch: String,
    /// Number of dirty paths reported by `git status --porcelain`.
    pub dirty: usize,
    /// Commits ahead/behind the upstream; `None` when no upstream is set.
    pub ahead_behind: Option<(usize, usize)>,
}

impl GitSummary {
    /// One-line render used by the repo list, e.g. `main *3 ↑1↓2`.
    pub fn describe(&self) -> String {
        let mut out = self.branch.clone();
        if self.dirty > 0 {
            out.push_str(&format!(" *{}", self.dirty));
        }
        match self.ahead_behind {
            Some((0, 0)) => {}
            Some((ahead, behind)) => {
                out.push(' ');
                if ahead > 0 {
                    out.push_str(&format!("↑{ahead}"));
                }
                if behind > 0 {
                    out.push_str(&format!("↓{behind}"));
                }
            }
            None => out.push_str(" (no upstream)"),
        }
        out
    }
}

/// Read the current branch, dirty count, and ahead/behind counts for `repo`.
pub fn summarize(repo: &Path) -> Result<GitSummary, String> {
    let branch = run_ok(repo, &["rev-parse", "--abbrev-ref", "HEAD"])?
        .trim()
        .to_string();
    let dirty = run_ok(repo, &["status", "--porcelain"])?
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count();
    // Fails when no upstream is configured; that's an expected state.
    let ahead_behind = run_ok(
        repo,
        &["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
    )
    .ok()
    .and_then(|raw| {
        let mut parts = raw.split_whitespace();
        let ahead = parts.next()?.parse().ok()?;
        let behind = parts.next()?.parse().ok()?;
        Some((ahead, behind))
    });
    Ok(GitSummary {
        branch,
        dirty,
        ahead_behind,
    })
}

/// Run `operation` against `repo`, returning a one-line status message
/// describing the steps taken, or the first failing step's error output.
pub fn run_operation(repo: &Path, operation: GitOperation) -> Result<String, String> {
    let mut steps = Vec::new();
    match operation {
        GitOperation::Sync => {
            if commit_all(repo)? {
                steps.push("committed");
            }
            pull_rebase(repo)?;
            steps.push("pulled");
            push(repo)?;
            steps.push("pushed");
        }
        GitOperation::Push => {
            if commit_all(repo)? {
                steps.push("committed");
            }
            push(repo)?;
            steps.push("pushed");
        }
        GitOperation::Pull => {
            pull_rebase(repo)?;
            steps.push("pulled");
        }
    }
    Ok(steps.join(", "))
}

/// Stage everything and commit when the index has changes. Returns `true` when
/// a commit was created.
fn commit_all(repo: &Path) -> Result<bool, String> {
    run_ok(repo, &["add", "-A"])?;
    // `diff --cached --quiet` exits 1 when there is something to commit.
    let staged = run(repo, &["diff", "--cached", "--quiet"])
        .map_err(|err| err.to_string())?
        .status
        .code()
        != Some(0);
    if !staged {
        return Ok(false);
    }
    run_ok(repo, &["commit", "-m", "pb repo: sync snippets"])?;
    Ok(true)
}

fn pull_rebase(repo: &Path) -> Result<(), String> {
    if let Err(err) = run_ok(repo, &["pull", "--rebase"]) {
        // A conflicting rebase would leave the repo mid-rebase under a TUI
        // where the user can't resolve it; roll back and report instead.
        let _ = run(repo, &["rebase", "--abort"]);
        return Err(err);
    }
    Ok(())
}

fn push(repo: &Path) -> Result<(), String> {
    run_ok(repo, &["push"]).map(|_| ())
}

fn run(repo: &Path, args: &[&str]) -> io::Result<Output> {
    Command::new("git").args(args).current_dir(repo).output()
}

/// Run git and return trimmed stdout, or a compact `git <verb>: <stderr>`
/// error string on a non-zero exit.
fn run_ok(repo: &Path, args: &[&str]) -> Result<String, String> {
    let output = run(repo, args).map_err(|err| format!("git {}: {err}", args[0]))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = stderr
            .lines()
            .find(|line| !line.trim().is_empty())
            .unwrap_or("failed")
            .trim();
        return Err(format!("git {}: {detail}", args[0]));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::PathBuf;

    fn temp_dir(prefix: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};

        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "pb-repo-git-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn git(dir: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir)
            .env("GIT_AUTHOR_NAME", "pb-test")
            .env("GIT_AUTHOR_EMAIL", "pb@test")
            .env("GIT_COMMITTER_NAME", "pb-test")
            .env("GIT_COMMITTER_EMAIL", "pb@test")
            .output()
            .expect("git runs");
        assert!(
            output.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    /// Create a bare "remote", a clone with one commit pushed, and a second
    /// clone to simulate another machine.
    fn remote_and_clone(workspace: &Path) -> (PathBuf, PathBuf) {
        let remote = workspace.join("remote.git");
        let clone = workspace.join("clone");
        git(workspace, &["init", "--bare", "-b", "main", "remote.git"]);
        git(workspace, &["clone", remote.to_str().unwrap(), "clone"]);
        git(&clone, &["checkout", "-b", "main"]);
        git(&clone, &["config", "user.name", "pb-test"]);
        git(&clone, &["config", "user.email", "pb@test"]);
        fs::write(clone.join("snippets.md"), "## Echo\n\n```\necho hi\n```\n").unwrap();
        git(&clone, &["add", "-A"]);
        git(&clone, &["commit", "-m", "init"]);
        git(&clone, &["push", "-u", "origin", "main"]);
        (remote, clone)
    }

    #[test]
    fn summarize_reports_branch_dirty_and_upstream() {
        let workspace = temp_dir("summary");
        let (_remote, clone) = remote_and_clone(&workspace);

        let clean = summarize(&clone).unwrap();
        assert_eq!(clean.branch, "main");
        assert_eq!(clean.dirty, 0);
        assert_eq!(clean.ahead_behind, Some((0, 0)));
        assert_eq!(clean.describe(), "main");

        fs::write(clone.join("new.md"), "## New\n\n```\ntrue\n```\n").unwrap();
        let dirty = summarize(&clone).unwrap();
        assert_eq!(dirty.dirty, 1);
        assert_eq!(dirty.describe(), "main *1");

        let _ = fs::remove_dir_all(&workspace);
    }

    #[test]
    fn sync_commits_pulls_and_pushes() {
        let workspace = temp_dir("sync");
        let (remote, clone) = remote_and_clone(&workspace);
        // A second clone pushes a commit the first must pull during sync.
        let other = workspace.join("other");
        git(&workspace, &["clone", remote.to_str().unwrap(), "other"]);
        git(&other, &["config", "user.name", "pb-test"]);
        git(&other, &["config", "user.email", "pb@test"]);
        fs::write(other.join("from-other.md"), "## Other\n\n```\ntrue\n```\n").unwrap();
        git(&other, &["add", "-A"]);
        git(&other, &["commit", "-m", "other"]);
        git(&other, &["push"]);

        fs::write(clone.join("local.md"), "## Local\n\n```\ntrue\n```\n").unwrap();
        let message = run_operation(&clone, GitOperation::Sync).unwrap();

        assert_eq!(message, "committed, pulled, pushed");
        assert!(clone.join("from-other.md").exists());
        let summary = summarize(&clone).unwrap();
        assert_eq!(summary.dirty, 0);
        assert_eq!(summary.ahead_behind, Some((0, 0)));

        let _ = fs::remove_dir_all(&workspace);
    }

    #[test]
    fn push_without_changes_only_pushes_and_pull_reports() {
        let workspace = temp_dir("push-pull");
        let (_remote, clone) = remote_and_clone(&workspace);

        assert_eq!(run_operation(&clone, GitOperation::Push).unwrap(), "pushed");
        assert_eq!(run_operation(&clone, GitOperation::Pull).unwrap(), "pulled");

        let _ = fs::remove_dir_all(&workspace);
    }

    #[test]
    fn failed_operation_surfaces_git_error() {
        let workspace = temp_dir("fail");
        let repo = workspace.join("norepo");
        fs::create_dir_all(&repo).unwrap();
        git(&repo, &["init", "-b", "main"]);
        git(&repo, &["config", "user.name", "pb-test"]);
        git(&repo, &["config", "user.email", "pb@test"]);

        // No remote configured: pull must fail with a git error message.
        let err = run_operation(&repo, GitOperation::Pull).unwrap_err();
        assert!(err.starts_with("git pull:"), "unexpected error: {err}");

        let _ = fs::remove_dir_all(&workspace);
    }
}
