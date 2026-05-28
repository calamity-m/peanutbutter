//! Suppression filtering — populates suppress paths from parsed files and
//! evaluates per-rule `disable`, `ignore_file`, and `ignore_command` globs.

use crate::config::LintConfig;

use super::{FileContext, LintFinding};

/// Populate `suppress_path` on each finding from the corresponding parsed file's
/// relative path. Findings with no matching file (e.g. state-file findings) are
/// left unchanged.
pub(super) fn attach_suppress_paths(findings: &mut [LintFinding], files: &[FileContext]) {
    for finding in findings {
        if finding.suppress_path.is_some() {
            continue;
        }
        if let Some(file) = files.iter().find(|file| file.path == finding.path) {
            finding.suppress_path = Some(
                file.parsed
                    .relative_path
                    .to_string_lossy()
                    .replace('\\', "/"),
            );
        }
    }
}

/// Return `true` if `finding` is suppressed by any rule in `config`.
pub(super) fn is_suppressed(finding: &LintFinding, config: &LintConfig) -> bool {
    let key = finding.code.strip_prefix("lint/").unwrap_or(finding.code);
    let Some(rule) = config.get(key) else {
        return false;
    };
    rule.disable
        || finding.suppress_path.as_deref().is_some_and(|path| {
            rule.ignore_file
                .iter()
                .any(|pattern| glob_matches(pattern, path))
        })
        || finding.suppress_command.as_deref().is_some_and(|command| {
            rule.ignore_command
                .iter()
                .any(|pattern| glob_matches(pattern, command))
        })
}

fn glob_matches(pattern: &str, value: &str) -> bool {
    glob_matches_bytes(pattern.as_bytes(), value.as_bytes())
}

fn glob_matches_bytes(pattern: &[u8], value: &[u8]) -> bool {
    match (pattern, value) {
        ([], []) => true,
        ([], _) => false,
        ([b'*', rest @ ..], _) => {
            glob_matches_bytes(rest, value)
                || (!value.is_empty() && glob_matches_bytes(pattern, &value[1..]))
        }
        ([b'?', rest @ ..], [_, value_rest @ ..]) => glob_matches_bytes(rest, value_rest),
        ([p, rest @ ..], [v, value_rest @ ..]) if p == v => glob_matches_bytes(rest, value_rest),
        _ => false,
    }
}
