//! Read-only snippet linting.

use crate::config::{AppConfig, Paths, SuggestionCommandsConfig, VariableInputConfig};
use crate::domain::{SnippetFile, SnippetId, VariableSource, VariableSpec};
use crate::{discovery, parser};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Broken or unterminated file frontmatter.
pub const CODE_BROKEN_FRONTMATTER: &str = "lint/broken-frontmatter";
/// Strict-mode placeholder has no inline, file-local, config, or built-in source.
pub const CODE_UNDECLARED_VARIABLE: &str = "lint/undeclared-variable";
/// Frontmatter or config variable definition is not referenced by snippets.
pub const CODE_UNUSED_VARIABLE: &str = "lint/unused-variable";
/// Two snippets in the same file slugify to the same base slug.
pub const CODE_DUPLICATE_SLUG: &str = "lint/duplicate-slug";
/// A suggestion command exited unsuccessfully.
pub const CODE_SUGGESTION_COMMAND_FAILED: &str = "lint/suggestion-command-failed";
/// A suggestion command exceeded the configured timeout.
pub const CODE_SUGGESTION_COMMAND_TIMEOUT: &str = "lint/suggestion-command-timeout";
/// Suggestion commands are disabled, so a command-backed variable was skipped.
pub const CODE_SUGGESTION_COMMANDS_DISABLED: &str = "lint/suggestion-commands-disabled";
/// Frecency GC found an orphan that has a likely current snippet candidate.
pub const CODE_GC_ORPHAN_REATTACHABLE: &str = "lint/gc-orphan-reattachable";
/// Frecency GC found an orphan with no likely current snippet candidate.
pub const CODE_GC_ORPHAN_UNRESOLVABLE: &str = "lint/gc-orphan-unresolvable";
/// Inline command appears to be a static suggestion list.
pub const CODE_STATIC_INLINE_COMMAND: &str = "lint/static-inline-command";
/// Strict markdown/snippet structure finding.
pub const CODE_MARKDOWN_STRUCTURE: &str = "lint/markdown-structure";
/// Strict snippet-defining code fence has no language tag.
pub const CODE_MISSING_CODE_LANGUAGE: &str = "lint/missing-code-language";
/// Strict file-local variable changes a global variable's suggestion source.
pub const CODE_FRONTMATTER_OVERRIDE: &str = "lint/frontmatter-override";

/// Runtime options for [`run`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LintOptions {
    /// Include opt-in style and structure checks.
    pub strict: bool,
    /// Render parseable JSON instead of human-oriented text.
    pub json: bool,
}

/// Severity attached to a lint finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LintSeverity {
    /// A finding likely to produce broken runtime behavior.
    Error,
    /// A finding worth fixing but not necessarily fatal at runtime.
    Warning,
}

/// A single lint diagnostic, shared by pretty and JSON renderers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LintFinding {
    /// Finding severity.
    pub severity: LintSeverity,
    /// Stable machine-readable code.
    pub code: &'static str,
    /// Snippet source path, state path, or other relevant path.
    pub path: PathBuf,
    /// Optional 1-based source line.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
    /// Optional snippet id when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet_id: Option<String>,
    /// Short user-facing message.
    pub message: String,
    /// Optional longer context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

/// Result of a lint run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LintResult {
    /// All findings sorted by path, line, code, and message.
    pub findings: Vec<LintFinding>,
}

impl LintResult {
    /// Returns `true` when at least one finding was reported.
    pub fn has_findings(&self) -> bool {
        !self.findings.is_empty()
    }
}

struct FileContext {
    root: PathBuf,
    path: PathBuf,
    content: String,
    parsed: SnippetFile,
}

/// Run lint over the configured snippet roots and write the selected output
/// format to `writer`. This function is read-only: suggestion commands may be
/// executed, but frecency GC is queried without saving or mutating state.
pub fn run<W: Write>(
    config: &AppConfig,
    options: LintOptions,
    writer: &mut W,
) -> io::Result<LintResult> {
    validate_roots(&config.paths.snippet_roots)?;
    let mut findings = Vec::new();
    let mut files = Vec::new();

    for root in &config.paths.snippet_roots {
        for path in discovery::discover_markdown_files(root)? {
            let content = match fs::read_to_string(&path) {
                Ok(content) => content,
                Err(err) => {
                    findings.push(finding(
                        LintSeverity::Warning,
                        CODE_MARKDOWN_STRUCTURE,
                        path,
                        None,
                        None,
                        "could not read snippet file".to_string(),
                        Some(err.to_string()),
                    ));
                    continue;
                }
            };
            let parsed = parser::parse_file(&path, root, &content);
            files.push(FileContext {
                root: root.clone(),
                path,
                content,
                parsed,
            });
        }
    }

    for file in &files {
        findings.extend(lint_frontmatter_source(&file.path, &file.content));
        findings.extend(lint_duplicate_slugs(&file.path, &file.content));
        findings.extend(lint_unused_file_variables(file));
        findings.extend(lint_suggestion_commands(
            file,
            &config.variables,
            &config.suggestion_commands,
        ));
        findings.extend(lint_static_inline_commands(file));
        if options.strict {
            findings.extend(lint_variables(file, &config.variables));
            findings.extend(lint_markdown_structure(&file.path, &file.content));
            findings.extend(lint_missing_code_languages(file));
            findings.extend(lint_frontmatter_overrides(file, &config.variables));
        }
    }

    findings.extend(lint_unused_config_variables(
        &config.variables,
        &config.paths.config_file,
        &files,
    ));
    let index =
        crate::index::SnippetIndex::from_files(files.iter().map(|file| file.parsed.clone()));
    findings.extend(lint_gc(&config.paths, &index)?);
    sort_findings(&mut findings);
    let result = LintResult { findings };
    if options.json {
        render_json(&result, writer)?;
    } else {
        render_pretty(&result, writer)?;
    }
    Ok(result)
}

fn validate_roots(roots: &[PathBuf]) -> io::Result<()> {
    for root in roots {
        match fs::metadata(root) {
            Ok(meta) if meta.is_dir() || meta.is_file() => {}
            Ok(_) => {
                return Err(io::Error::other(format!(
                    "snippet root is not a file or directory: {}",
                    root.display()
                )));
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => {
                return Err(io::Error::new(
                    err.kind(),
                    format!("unreadable snippet root {}: {err}", root.display()),
                ));
            }
        }
    }
    Ok(())
}

fn render_json<W: Write>(result: &LintResult, writer: &mut W) -> io::Result<()> {
    serde_json::to_writer_pretty(&mut *writer, result).map_err(io::Error::other)?;
    writeln!(writer)
}

fn render_pretty<W: Write>(result: &LintResult, writer: &mut W) -> io::Result<()> {
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

fn lint_frontmatter_source(path: &Path, content: &str) -> Vec<LintFinding> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.first().map(|l| l.trim()) != Some("---") {
        return Vec::new();
    }
    let Some(end) = lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, line)| line.trim() == "---")
        .map(|(idx, _)| idx)
    else {
        return vec![finding(
            LintSeverity::Error,
            CODE_BROKEN_FRONTMATTER,
            path.to_path_buf(),
            Some(1),
            None,
            "frontmatter block is not terminated".to_string(),
            None,
        )];
    };
    let mut out = Vec::new();
    let mut i = 1;
    while i < end {
        let trimmed = lines[i].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }
        if !trimmed.contains(':') && !trimmed.starts_with('-') {
            out.push(finding(
                LintSeverity::Error,
                CODE_BROKEN_FRONTMATTER,
                path.to_path_buf(),
                Some(i + 1),
                None,
                "frontmatter line is not a supported key/value entry".to_string(),
                None,
            ));
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            if key.trim().is_empty() {
                out.push(finding(
                    LintSeverity::Error,
                    CODE_BROKEN_FRONTMATTER,
                    path.to_path_buf(),
                    Some(i + 1),
                    None,
                    "frontmatter key is empty".to_string(),
                    None,
                ));
            }
            if value.trim().starts_with('[') && !value.trim().ends_with(']') {
                out.push(finding(
                    LintSeverity::Error,
                    CODE_BROKEN_FRONTMATTER,
                    path.to_path_buf(),
                    Some(i + 1),
                    None,
                    "inline list is not closed".to_string(),
                    None,
                ));
            }
        }
        i += 1;
    }
    out
}

fn lint_duplicate_slugs(path: &Path, content: &str) -> Vec<LintFinding> {
    let mut first: HashMap<String, usize> = HashMap::new();
    let mut out = Vec::new();
    for (idx, line) in content.lines().enumerate() {
        if let Some(heading) = snippet_heading(line) {
            let slug = slugify(&heading);
            if let Some(first_line) = first.get(&slug) {
                out.push(finding(
                    LintSeverity::Warning,
                    CODE_DUPLICATE_SLUG,
                    path.to_path_buf(),
                    Some(idx + 1),
                    None,
                    format!("snippet heading duplicates base slug '{slug}'"),
                    Some(format!("first occurrence is on line {first_line}")),
                ));
            } else {
                first.insert(slug, idx + 1);
            }
        }
    }
    out
}

fn lint_variables(
    file: &FileContext,
    globals: &BTreeMap<String, VariableInputConfig>,
) -> Vec<LintFinding> {
    let ranges = parser::snippet_line_ranges(&file.parsed.relative_path, &file.content);
    let mut line_by_id = HashMap::new();
    for range in ranges {
        line_by_id.insert(range.id, range.start_line + 1);
    }
    let mut out = Vec::new();
    for snippet in &file.parsed.snippets {
        let mut seen = HashSet::new();
        for variable in &snippet.variables {
            if !seen.insert(variable.name.clone())
                || !matches!(variable.source, VariableSource::Free)
            {
                continue;
            }
            if is_builtin_variable(&variable.name)
                || file
                    .parsed
                    .frontmatter
                    .variables
                    .contains_key(&variable.name)
                || globals.contains_key(&variable.name)
            {
                continue;
            }
            out.push(finding(LintSeverity::Warning, CODE_UNDECLARED_VARIABLE, file.path.clone(), line_by_id.get(&snippet.id).copied(), Some(snippet.id.clone()), format!("variable '{}' has no declared source", variable.name), Some("add an inline default/command, a file-local frontmatter variable, or a global variable config if this is not intentionally manual input".to_string())));
        }
    }
    out
}

fn lint_unused_file_variables(file: &FileContext) -> Vec<LintFinding> {
    let referenced = referenced_variables(file);
    let lines = frontmatter_variable_lines(&file.content);
    file.parsed
        .frontmatter
        .variables
        .keys()
        .filter(|name| !referenced.contains(*name))
        .map(|name| {
            finding(
                LintSeverity::Warning,
                CODE_UNUSED_VARIABLE,
                file.path.clone(),
                lines.get(name).copied(),
                None,
                format!(
                    "frontmatter variable '{name}' is not referenced by any snippet in this file"
                ),
                Some(
                    "remove the variable definition or add a matching <@name> placeholder"
                        .to_string(),
                ),
            )
        })
        .collect()
}

fn lint_unused_config_variables(
    globals: &BTreeMap<String, VariableInputConfig>,
    config_file: &Path,
    files: &[FileContext],
) -> Vec<LintFinding> {
    let mut referenced = HashSet::new();
    for file in files {
        referenced.extend(referenced_variables(file));
    }
    globals
        .keys()
        .filter(|name| !referenced.contains(*name))
        .map(|name| {
            finding(
                LintSeverity::Warning,
                CODE_UNUSED_VARIABLE,
                config_file.to_path_buf(),
                None,
                None,
                format!("config variable '{name}' is not referenced by any snippet"),
                Some(
                    "remove the variable definition or add a matching <@name> placeholder"
                        .to_string(),
                ),
            )
        })
        .collect()
}

fn referenced_variables(file: &FileContext) -> HashSet<String> {
    file.parsed
        .snippets
        .iter()
        .flat_map(|snippet| {
            snippet
                .variables
                .iter()
                .map(|variable| variable.name.clone())
        })
        .collect()
}

fn frontmatter_variable_lines(content: &str) -> HashMap<String, usize> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.first().map(|line| line.trim()) != Some("---") {
        return HashMap::new();
    }
    let Some(end) = lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, line)| line.trim() == "---")
        .map(|(idx, _)| idx)
    else {
        return HashMap::new();
    };

    let mut out = HashMap::new();
    let Some(variables_line) = lines[..end]
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, line)| line.trim() == "variables:")
        .map(|(idx, _)| idx)
    else {
        return out;
    };

    for (idx, line) in lines.iter().enumerate().take(end).skip(variables_line + 1) {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if indent == 0 || trimmed.is_empty() || trimmed.starts_with('#') {
            break;
        }
        if let Some((name, rest)) = trimmed.split_once(':')
            && rest.trim().is_empty()
        {
            out.insert(name.trim().to_string(), idx + 1);
        }
    }
    out
}

fn lint_suggestion_commands(
    file: &FileContext,
    globals: &BTreeMap<String, VariableInputConfig>,
    config: &SuggestionCommandsConfig,
) -> Vec<LintFinding> {
    let mut out = Vec::new();
    for snippet in &file.parsed.snippets {
        let commands = command_sources(snippet, &file.parsed.frontmatter.variables, globals);
        for (name, command) in commands {
            if !config.allow_commands {
                out.push(finding(LintSeverity::Warning, CODE_SUGGESTION_COMMANDS_DISABLED, file.path.clone(), None, Some(snippet.id.clone()), format!("suggestion command for variable '{name}' was skipped because commands are disabled"), None));
                continue;
            }
            match crate::execute::command_suggestions(&command, &file.root, config.timeout_ms) {
                Ok(_) => {}
                Err(err) => {
                    let msg = err.to_string();
                    let code = if msg.contains("timed out") {
                        CODE_SUGGESTION_COMMAND_TIMEOUT
                    } else {
                        CODE_SUGGESTION_COMMAND_FAILED
                    };
                    out.push(finding(
                        LintSeverity::Error,
                        code,
                        file.path.clone(),
                        None,
                        Some(snippet.id.clone()),
                        format!("suggestion command for variable '{name}' failed"),
                        Some(msg),
                    ));
                }
            }
        }
    }
    out
}

fn command_sources(
    snippet: &crate::domain::Snippet,
    locals: &BTreeMap<String, VariableSpec>,
    globals: &BTreeMap<String, VariableInputConfig>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for variable in &snippet.variables {
        if !seen.insert(variable.name.clone()) {
            continue;
        }
        match &variable.source {
            VariableSource::Command(command) => out.push((variable.name.clone(), command.clone())),
            VariableSource::Free => {
                if let Some(command) = locals
                    .get(&variable.name)
                    .and_then(|spec| spec.command.clone())
                    .or_else(|| {
                        globals
                            .get(&variable.name)
                            .and_then(|spec| spec.command.clone())
                    })
                {
                    out.push((variable.name.clone(), command));
                }
            }
            VariableSource::Default(_) => {}
        }
    }
    out
}

fn lint_static_inline_commands(file: &FileContext) -> Vec<LintFinding> {
    let mut out = Vec::new();
    for snippet in &file.parsed.snippets {
        for variable in &snippet.variables {
            let VariableSource::Command(command) = &variable.source else {
                continue;
            };
            if looks_static_command(command) {
                out.push(finding(
                    LintSeverity::Warning,
                    CODE_STATIC_INLINE_COMMAND,
                    file.path.clone(),
                    None,
                    Some(snippet.id.clone()),
                    format!(
                        "inline command for variable '{}' looks like a static suggestion list",
                        variable.name
                    ),
                    Some(
                        "prefer frontmatter variables.<name>.suggestions for static lists"
                            .to_string(),
                    ),
                ));
            }
        }
    }
    out
}

fn lint_markdown_structure(path: &Path, content: &str) -> Vec<LintFinding> {
    let mut out = Vec::new();
    let mut in_fence: Option<(String, usize)> = None;
    let mut open_heading: Option<(String, usize, bool)> = None;
    for (idx, line) in content.lines().enumerate() {
        let line_no = idx + 1;
        if let Some((fence, _)) = &in_fence {
            if is_fence_close(line, fence) {
                in_fence = None;
                if let Some((_, _, has_code)) = &mut open_heading {
                    *has_code = true;
                }
            }
            continue;
        }
        if let Some(heading) = snippet_heading(line) {
            if let Some((previous, previous_line, false)) =
                open_heading.replace((heading, line_no, false))
            {
                out.push(finding(
                    LintSeverity::Warning,
                    CODE_MARKDOWN_STRUCTURE,
                    path.to_path_buf(),
                    Some(previous_line),
                    None,
                    format!("snippet section '{previous}' has no code fence"),
                    None,
                ));
            }
        } else if let Some(fence) = fence_open(line) {
            in_fence = Some((fence, line_no));
        }
    }
    if let Some((_, start)) = in_fence {
        out.push(finding(
            LintSeverity::Warning,
            CODE_MARKDOWN_STRUCTURE,
            path.to_path_buf(),
            Some(start),
            None,
            "code fence is not closed".to_string(),
            None,
        ));
    }
    if let Some((heading, line, false)) = open_heading {
        out.push(finding(
            LintSeverity::Warning,
            CODE_MARKDOWN_STRUCTURE,
            path.to_path_buf(),
            Some(line),
            None,
            format!("snippet section '{heading}' has no code fence"),
            None,
        ));
    }
    out
}

fn lint_missing_code_languages(file: &FileContext) -> Vec<LintFinding> {
    let ranges = parser::snippet_line_ranges(&file.parsed.relative_path, &file.content);
    let mut line_by_id = HashMap::new();
    for range in ranges {
        line_by_id.insert(range.id, range.start_line + 1);
    }
    file.parsed
        .snippets
        .iter()
        .filter(|snippet| snippet.language.is_none())
        .map(|snippet| {
            finding(
                LintSeverity::Warning,
                CODE_MISSING_CODE_LANGUAGE,
                file.path.clone(),
                line_by_id.get(&snippet.id).copied(),
                Some(snippet.id.clone()),
                "snippet code fence has no language tag".to_string(),
                None,
            )
        })
        .collect()
}

fn lint_frontmatter_overrides(
    file: &FileContext,
    globals: &BTreeMap<String, VariableInputConfig>,
) -> Vec<LintFinding> {
    let mut out = Vec::new();
    for (name, local) in &file.parsed.frontmatter.variables {
        let Some(global) = globals.get(name) else {
            continue;
        };
        let local_source = suggestion_source(local);
        let global_source = suggestion_source(global);
        if local_source != global_source && local_source != "none" && global_source != "none" {
            out.push(finding(
                LintSeverity::Warning,
                CODE_FRONTMATTER_OVERRIDE,
                file.path.clone(),
                Some(1),
                None,
                format!("file-local variable '{name}' overrides a global suggestion source"),
                Some("rename the local variable if it means something different".to_string()),
            ));
        }
    }
    out
}

fn lint_gc(paths: &Paths, index: &crate::index::SnippetIndex) -> io::Result<Vec<LintFinding>> {
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

fn finding(
    severity: LintSeverity,
    code: &'static str,
    path: PathBuf,
    line: Option<usize>,
    snippet_id: Option<SnippetId>,
    message: String,
    detail: Option<String>,
) -> LintFinding {
    LintFinding {
        severity,
        code,
        path,
        line,
        snippet_id: snippet_id.map(|id| id.to_string()),
        message,
        detail,
    }
}

fn sort_findings(findings: &mut [LintFinding]) {
    findings.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line.cmp(&b.line))
            .then(a.code.cmp(b.code))
            .then(a.message.cmp(&b.message))
    });
}

fn is_builtin_variable(name: &str) -> bool {
    matches!(name, "file" | "directory")
}

fn snippet_heading(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("##")?;
    if rest.starts_with('#') {
        return None;
    }
    let text = rest.trim();
    (!text.is_empty()).then(|| text.to_string())
}

fn fence_open(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with("```") {
        return None;
    }
    let ticks: String = trimmed.chars().take_while(|c| *c == '`').collect();
    (ticks.len() >= 3).then_some(ticks)
}

fn is_fence_close(line: &str, fence: &str) -> bool {
    let trimmed = line.trim();
    trimmed.starts_with(fence) && trimmed.chars().all(|c| c == '`') && trimmed.len() >= fence.len()
}

fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_dash = true;
    for c in input.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        "snippet".to_string()
    } else {
        out
    }
}

fn looks_static_command(command: &str) -> bool {
    let trimmed = command.trim();
    trimmed.starts_with("echo ") || trimmed.starts_with("printf ")
}

fn suggestion_source(spec: &VariableSpec) -> &'static str {
    if spec.command.is_some() {
        "command"
    } else if !spec.suggestions.is_empty() {
        "suggestions"
    } else {
        "none"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{FrecencyConfig, FuzzyWeights, SearchConfig, Theme, UiConfig};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir(prefix: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "pb-lint-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn config(root: PathBuf) -> AppConfig {
        AppConfig {
            paths: Paths {
                snippet_roots: vec![root.clone()],
                state_file: root.join("state.tsv"),
                config_file: root.join("config.toml"),
            },
            ui: UiConfig::default(),
            search: SearchConfig {
                frecency_weight: 250.0,
                fuzzy: FuzzyWeights::default(),
                frecency: FrecencyConfig::default(),
            },
            variables: BTreeMap::new(),
            theme: Theme::default(),
            suggestion_commands: SuggestionCommandsConfig {
                timeout_ms: 50,
                allow_commands: true,
            },
        }
    }

    #[test]
    fn undeclared_variable_is_strict_warning_and_json_is_parseable() {
        let root = temp_dir("undeclared");
        fs::write(
            root.join("snippets.md"),
            "## Demo\n\n```bash\necho <@name>\n```\n",
        )
        .unwrap();
        let mut out = Vec::new();
        let normal = run(
            &config(root.clone()),
            LintOptions {
                strict: false,
                json: true,
            },
            &mut out,
        )
        .unwrap();
        assert!(
            !normal
                .findings
                .iter()
                .any(|f| f.code == CODE_UNDECLARED_VARIABLE)
        );
        let json: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert!(json["findings"].is_array());

        out.clear();
        let strict = run(
            &config(root),
            LintOptions {
                strict: true,
                json: false,
            },
            &mut out,
        )
        .unwrap();
        let finding = strict
            .findings
            .iter()
            .find(|f| f.code == CODE_UNDECLARED_VARIABLE)
            .unwrap();
        assert_eq!(finding.severity, LintSeverity::Warning);
    }

    #[test]
    fn unused_frontmatter_variable_is_reported() {
        let root = temp_dir("unused-frontmatter");
        fs::write(
            root.join("snippets.md"),
            "---\nvariables:\n  unused:\n    suggestions: [a]\n  used:\n    suggestions: [b]\n---\n\n## Demo\n\n```bash\necho <@used>\n```\n",
        )
        .unwrap();
        let mut out = Vec::new();
        let result = run(
            &config(root),
            LintOptions {
                strict: false,
                json: false,
            },
            &mut out,
        )
        .unwrap();
        assert!(result.findings.iter().any(|finding| {
            finding.code == CODE_UNUSED_VARIABLE
                && finding.message.contains("frontmatter variable 'unused'")
        }));
        assert!(!result.findings.iter().any(|finding| {
            finding.code == CODE_UNUSED_VARIABLE && finding.message.contains("'used'")
        }));
    }

    #[test]
    fn unused_config_variable_is_reported() {
        let root = temp_dir("unused-config");
        fs::write(
            root.join("snippets.md"),
            "## Demo\n\n```bash\necho <@used>\n```\n",
        )
        .unwrap();
        let mut cfg = config(root);
        cfg.variables.insert(
            "used".to_string(),
            VariableSpec {
                suggestions: vec!["a".to_string()],
                ..VariableSpec::default()
            },
        );
        cfg.variables.insert(
            "unused".to_string(),
            VariableSpec {
                suggestions: vec!["b".to_string()],
                ..VariableSpec::default()
            },
        );
        let mut out = Vec::new();
        let result = run(
            &cfg,
            LintOptions {
                strict: false,
                json: false,
            },
            &mut out,
        )
        .unwrap();
        assert!(result.findings.iter().any(|finding| {
            finding.code == CODE_UNUSED_VARIABLE
                && finding.message.contains("config variable 'unused'")
        }));
        assert!(!result.findings.iter().any(|finding| {
            finding.code == CODE_UNUSED_VARIABLE && finding.message.contains("'used'")
        }));
    }

    #[test]
    fn strict_missing_language_is_opt_in() {
        let root = temp_dir("strict");
        fs::write(root.join("snippets.md"), "## Demo\n\n```\necho ok\n```\n").unwrap();
        let mut out = Vec::new();
        let normal = run(
            &config(root.clone()),
            LintOptions {
                strict: false,
                json: false,
            },
            &mut out,
        )
        .unwrap();
        assert!(
            !normal
                .findings
                .iter()
                .any(|f| f.code == CODE_MISSING_CODE_LANGUAGE)
        );
        out.clear();
        let strict = run(
            &config(root),
            LintOptions {
                strict: true,
                json: false,
            },
            &mut out,
        )
        .unwrap();
        assert!(
            strict
                .findings
                .iter()
                .any(|f| f.code == CODE_MISSING_CODE_LANGUAGE)
        );
    }
}
