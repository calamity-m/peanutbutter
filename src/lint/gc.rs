//! GC orphan lint adapter — surfaces orphaned frecency IDs as lint findings
//! so they appear alongside snippet diagnostics.

use std::io;

use crate::config::Paths;
use crate::index::SnippetIndex;

use super::{
    CODE_GC_ORPHAN_REATTACHABLE, CODE_GC_ORPHAN_UNRESOLVABLE, LintFinding, LintSeverity, finding,
};

/// Collect frecency GC orphans and convert them to lint findings on the state file.
pub(super) fn lint_gc(paths: &Paths, index: &SnippetIndex) -> io::Result<Vec<LintFinding>> {
    let mut out = Vec::new();
    for orphan in crate::gc::collect_orphans_with_index(paths, index)? {
        let (code, detail) = match orphan.candidate_id {
            Some(candidate) => (
                CODE_GC_ORPHAN_REATTACHABLE,
                Some(format!("candidate: {candidate}")),
            ),
            None => (CODE_GC_ORPHAN_UNRESOLVABLE, None),
        };
        out.push(finding(
            LintSeverity::Warning,
            code,
            paths.state_file.clone(),
            None,
            Some(orphan.id),
            format!("orphaned frecency id has {} event(s)", orphan.events),
            detail,
        ));
    }
    Ok(out)
}
