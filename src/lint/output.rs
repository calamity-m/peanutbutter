//! Output rendering for lint results — JSON and human-readable pretty formats.

use std::io::{self, Write};
use std::path::Path;

use super::{LintResult, LintSeverity};

/// Serialize `result` as pretty-printed JSON followed by a newline.
pub(super) fn render_json<W: Write>(result: &LintResult, writer: &mut W) -> io::Result<()> {
    serde_json::to_writer_pretty(&mut *writer, result).map_err(io::Error::other)?;
    writeln!(writer)
}

/// Write a human-readable summary grouped by file path.
pub(super) fn render_pretty<W: Write>(result: &LintResult, writer: &mut W) -> io::Result<()> {
    if result.findings.is_empty() {
        writeln!(writer, "No lint findings.")?;
        return Ok(());
    }
    let mut current: Option<&Path> = None;
    for finding in &result.findings {
        if current != Some(finding.path.as_path()) {
            if current.is_some() {
                writeln!(writer)?;
            }
            writeln!(writer, "{}", finding.path.display())?;
            current = Some(&finding.path);
        }
        let sev = match finding.severity {
            LintSeverity::Error => "error",
            LintSeverity::Warning => "warning",
        };
        let line = finding.line.map(|l| format!(":{l}")).unwrap_or_default();
        let snippet = finding
            .snippet_id
            .as_deref()
            .map(|id| format!(" [{id}]"))
            .unwrap_or_default();
        writeln!(
            writer,
            "  {sev}{} {}{}: {}",
            line, finding.code, snippet, finding.message
        )?;
        if let Some(detail) = &finding.detail {
            writeln!(writer, "    {detail}")?;
        }
    }
    Ok(())
}
