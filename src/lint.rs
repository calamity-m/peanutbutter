//! Read-only snippet linting.

use crate::config::{AppConfig, LintConfig, Paths, SuggestionCommandsConfig, VariableInputConfig};
use crate::domain::{SnippetFile, SnippetId, VariableSource, VariableSpec};
use crate::{discovery, parser};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Broken or unterminated file frontmatter.
pub const CODE_BROKEN_FRONTMATTER: &str = "lint/broken-frontmatter";
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
/// Snippet section contains only `text` fences and no executable body.
pub const CODE_TEXT_ONLY_SECTION: &str = "lint/text-only-section";
/// Strict snippet-defining code fence has no language tag.
pub const CODE_MISSING_CODE_LANGUAGE: &str = "lint/missing-code-language";
/// Strict file-local variable changes a global variable's suggestion source.
pub const CODE_FRONTMATTER_OVERRIDE: &str = "lint/frontmatter-override";
/// A suggestion command contains `<#name>` references, so it was skipped at
/// lint time because there is no user input to substitute.
pub const CODE_DEPENDENT_SUGGESTION_SKIPPED: &str = "lint/dependent-suggestion-skipped";
/// `<#name>` references a variable that is not declared in this snippet or
/// in its frontmatter.
pub const CODE_UNKNOWN_VARIABLE_REFERENCE: &str = "lint/unknown-variable-reference";
/// `<#name>` references a variable that appears later in prompt order.
pub const CODE_FORWARD_VARIABLE_REFERENCE: &str = "lint/forward-variable-reference";
/// `<#name>` references the variable whose own command it sits inside.
pub const CODE_SELF_VARIABLE_REFERENCE: &str = "lint/self-variable-reference";
/// A suggestion command's `<#...>` syntax could not be parsed.
pub const CODE_INVALID_DEPENDENT_REFERENCE: &str = "lint/invalid-dependent-reference";

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
    /// Optional 0-based byte column where the finding begins on `line`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub col_start: Option<usize>,
    /// Optional 0-based byte column where the finding ends on `line`
    /// (exclusive).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub col_end: Option<usize>,
    /// Optional snippet id when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet_id: Option<String>,
    /// Short user-facing message.
    pub message: String,
    /// Optional longer context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip)]
    suppress_path: Option<String>,
    #[serde(skip)]
    suppress_command: Option<String>,
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
        findings.extend(lint_dependent_references(file, &config.variables));
        if options.strict {
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
    attach_suppress_paths(&mut findings, &files);
    findings.retain(|finding| !is_suppressed(finding, &config.lint));
    sort_findings(&mut findings);
    let result = LintResult { findings };
    if options.json {
        render_json(&result, writer)?;
    } else {
        render_pretty(&result, writer)?;
    }
    Ok(result)
}

/// Run single-file lint checks on in-memory content and return the findings.
///
/// This is intended for use by the LSP server, which re-parses documents on
/// each change without reading from disk. Cross-file checks (unused config
/// variables, duplicate slugs across files, GC) are not included.
pub fn lint_file(path: &Path, root: &Path, content: &str, config: &AppConfig) -> Vec<LintFinding> {
    let parsed = parser::parse_file(path, root, content);
    let ctx = FileContext {
        root: root.to_path_buf(),
        path: path.to_path_buf(),
        content: content.to_string(),
        parsed,
    };
    let mut findings = Vec::new();
    findings.extend(lint_frontmatter_source(path, content));
    findings.extend(lint_duplicate_slugs(path, content));
    findings.extend(lint_unused_file_variables(&ctx));
    findings.extend(lint_suggestion_commands(
        &ctx,
        &config.variables,
        &config.suggestion_commands,
    ));
    findings.extend(lint_static_inline_commands(&ctx));
    findings.extend(lint_dependent_references(&ctx, &config.variables));
    findings.extend(lint_markdown_structure(path, content));
    findings.extend(lint_missing_code_languages(&ctx));
    findings.extend(lint_frontmatter_overrides(&ctx, &config.variables));
    attach_suppress_paths(&mut findings, &[ctx]);
    findings.retain(|f| !is_suppressed(f, &config.lint));
    sort_findings(&mut findings);
    findings
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
            // If the command references upstream variables, skip execution —
            // there is no user input at lint time to substitute.
            match crate::command_template::parse_command_template(&command) {
                Ok(template) if crate::command_template::is_dependent(&template) => {
                    out.push(
                        finding(
                            LintSeverity::Warning,
                            CODE_DEPENDENT_SUGGESTION_SKIPPED,
                            file.path.clone(),
                            None,
                            Some(snippet.id.clone()),
                            format!(
                                "suggestion command for variable '{name}' was skipped because it references upstream variables"
                            ),
                            None,
                        )
                        .with_suppress_command(command.clone()),
                    );
                    continue;
                }
                Ok(_) => {}
                Err(err) => {
                    out.push(
                        finding(
                            LintSeverity::Error,
                            CODE_INVALID_DEPENDENT_REFERENCE,
                            file.path.clone(),
                            None,
                            Some(snippet.id.clone()),
                            format!(
                                "suggestion command for variable '{name}' has an invalid <# reference"
                            ),
                            Some(err.to_string()),
                        )
                        .with_suppress_command(command.clone()),
                    );
                    continue;
                }
            }
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
                    out.push(
                        finding(
                            LintSeverity::Error,
                            code,
                            file.path.clone(),
                            None,
                            Some(snippet.id.clone()),
                            format!("suggestion command for variable '{name}' failed"),
                            Some(msg),
                        )
                        .with_suppress_command(command.clone()),
                    );
                }
            }
        }
    }
    out
}

/// Find all `<#name>` token spans in `content`. Returns
/// `(name, line_1based, col_start, col_end)` tuples in document order.
/// Backslash-escaped `\<#...>` is skipped. The returned columns are 0-based
/// byte offsets within the line.
fn find_dependent_ref_spans(content: &str) -> Vec<(String, usize, usize, usize)> {
    let mut out = Vec::new();
    for (line_idx, line) in content.lines().enumerate() {
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            // Skip escaped \<#...>
            if bytes[i] == b'\\'
                && i + 2 < bytes.len()
                && bytes[i + 1] == b'<'
                && bytes[i + 2] == b'#'
            {
                if let Some(off) = line[i + 1..].find('>') {
                    i = i + 1 + off + 1;
                    continue;
                }
                i += 1;
                continue;
            }
            if bytes[i] == b'<' && i + 1 < bytes.len() && bytes[i + 1] == b'#' {
                let inner_start = i + 2;
                let Some(off) = line[inner_start..].find('>') else {
                    i += 1;
                    continue;
                };
                let inner_end = inner_start + off;
                let inner = &line[inner_start..inner_end];
                let name = inner.split(':').next().unwrap_or(inner).trim().to_string();
                if !name.is_empty() {
                    out.push((name, line_idx + 1, i, inner_end + 1));
                }
                i = inner_end + 1;
                continue;
            }
            i += 1;
        }
    }
    out
}

/// Walk all suggestion commands in `file` and emit lint findings for
/// `<#name>` references that are: unknown, forward, or self.
fn lint_dependent_references(
    file: &FileContext,
    globals: &BTreeMap<String, VariableInputConfig>,
) -> Vec<LintFinding> {
    use crate::command_template::{parse_command_template, referenced_names};
    let mut out = Vec::new();

    // Pre-scan the file content for all <#name> spans so we can attach
    // precise diagnostic ranges. We pop spans as we attribute findings to
    // them — if two refs of the same name exist, each one gets a position
    // in document order.
    let mut spans = find_dependent_ref_spans(&file.content);
    let mut take_span = |name: &str| -> Option<(usize, usize, usize)> {
        let pos = spans.iter().position(|(n, ..)| n == name)?;
        let (_, l, c0, c1) = spans.remove(pos);
        Some((l, c0, c1))
    };

    for snippet in &file.parsed.snippets {
        // Build prompt order from the deduplicated body variables. Forward
        // references are decided against this order.
        let mut prompt_order: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        for variable in &snippet.variables {
            if seen.insert(variable.name.clone()) {
                prompt_order.push(variable.name.clone());
            }
        }
        let order_index: HashMap<String, usize> = prompt_order
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), i))
            .collect();

        // The universe of declared variables: body, frontmatter, config.
        let mut declared: HashSet<String> = prompt_order.iter().cloned().collect();
        declared.extend(file.parsed.frontmatter.variables.keys().cloned());
        declared.extend(globals.keys().cloned());
        // Builtins are always declared.
        declared.insert("file".to_string());
        declared.insert("directory".to_string());

        for (owner, command) in
            command_sources(snippet, &file.parsed.frontmatter.variables, globals)
        {
            let Ok(template) = parse_command_template(&command) else {
                continue;
            };
            let refs = referenced_names(&template);
            for ref_name in &refs {
                let span = take_span(ref_name);
                if ref_name == &owner {
                    let mut f = finding(
                        LintSeverity::Error,
                        CODE_SELF_VARIABLE_REFERENCE,
                        file.path.clone(),
                        None,
                        Some(snippet.id.clone()),
                        format!("variable '{owner}' references itself via <#{ref_name}>"),
                        None,
                    )
                    .with_suppress_command(command.clone());
                    if let Some((l, c0, c1)) = span {
                        f = f.with_span(l, c0, c1);
                    }
                    out.push(f);
                    continue;
                }
                if !declared.contains(ref_name) {
                    let mut f = finding(
                        LintSeverity::Error,
                        CODE_UNKNOWN_VARIABLE_REFERENCE,
                        file.path.clone(),
                        None,
                        Some(snippet.id.clone()),
                        format!("variable '{owner}' references unknown <#{ref_name}>"),
                        Some("declare it with an inline placeholder, frontmatter variable, or config override".to_string()),
                    )
                    .with_suppress_command(command.clone());
                    if let Some((l, c0, c1)) = span {
                        f = f.with_span(l, c0, c1);
                    }
                    out.push(f);
                    continue;
                }
                if let (Some(&owner_idx), Some(&ref_idx)) =
                    (order_index.get(&owner), order_index.get(ref_name))
                    && ref_idx >= owner_idx
                {
                    let mut f = finding(
                        LintSeverity::Error,
                        CODE_FORWARD_VARIABLE_REFERENCE,
                        file.path.clone(),
                        None,
                        Some(snippet.id.clone()),
                        format!(
                            "variable '{owner}' references <#{ref_name}> which comes later in prompt order"
                        ),
                        None,
                    )
                    .with_suppress_command(command.clone());
                    if let Some((l, c0, c1)) = span {
                        f = f.with_span(l, c0, c1);
                    }
                    out.push(f);
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
                out.push(
                    finding(
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
                    )
                    .with_suppress_command(command.clone()),
                );
            }
        }
    }
    out
}

fn lint_markdown_structure(path: &Path, content: &str) -> Vec<LintFinding> {
    let mut out = Vec::new();
    // `in_fence` carries `(fence, line_no, is_text)` for the currently-open fence.
    let mut in_fence: Option<(String, usize, bool)> = None;
    // `open_heading` carries `(heading, line_no, has_executable, has_text)`.
    let mut open_heading: Option<(String, usize, bool, bool)> = None;
    for (idx, line) in content.lines().enumerate() {
        let line_no = idx + 1;
        if let Some((fence, _, is_text)) = &in_fence {
            if is_fence_close(line, fence) {
                let was_text = *is_text;
                in_fence = None;
                if let Some((_, _, has_executable, has_text)) = &mut open_heading {
                    if was_text {
                        *has_text = true;
                    } else {
                        *has_executable = true;
                    }
                }
            }
            continue;
        }
        if let Some(heading) = snippet_heading(line) {
            if let Some(previous) = open_heading.replace((heading, line_no, false, false)) {
                emit_section_structure_finding(&mut out, path, previous);
            }
        } else if let Some((fence, language)) = fence_open(line) {
            let is_text = is_ignored_body_language(language.as_deref());
            in_fence = Some((fence, line_no, is_text));
        }
    }
    if let Some((_, start, _)) = in_fence {
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
    if let Some(previous) = open_heading {
        emit_section_structure_finding(&mut out, path, previous);
    }
    out
}

fn emit_section_structure_finding(
    out: &mut Vec<LintFinding>,
    path: &Path,
    section: (String, usize, bool, bool),
) {
    let (heading, line, has_executable, has_text) = section;
    if has_executable {
        return;
    }
    if has_text {
        out.push(finding(
            LintSeverity::Warning,
            CODE_TEXT_ONLY_SECTION,
            path.to_path_buf(),
            Some(line),
            None,
            format!(
                "snippet section '{heading}' has only `text` fences; \
                 `text` is reserved for preview examples and is not executable"
            ),
            None,
        ));
    } else {
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
}

fn is_ignored_body_language(language: Option<&str>) -> bool {
    language.is_some_and(|language| language.eq_ignore_ascii_case("text"))
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
        col_start: None,
        col_end: None,
        snippet_id: snippet_id.map(|id| id.to_string()),
        message,
        detail,
        suppress_path: None,
        suppress_command: None,
    }
}

fn attach_suppress_paths(findings: &mut [LintFinding], files: &[FileContext]) {
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

impl LintFinding {
    fn with_suppress_command(mut self, command: String) -> Self {
        self.suppress_command = Some(command);
        self
    }

    fn with_span(mut self, line: usize, col_start: usize, col_end: usize) -> Self {
        self.line = Some(line);
        self.col_start = Some(col_start);
        self.col_end = Some(col_end);
        self
    }
}

fn is_suppressed(finding: &LintFinding, config: &LintConfig) -> bool {
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

fn sort_findings(findings: &mut [LintFinding]) {
    findings.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line.cmp(&b.line))
            .then(a.code.cmp(b.code))
            .then(a.message.cmp(&b.message))
    });
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

fn fence_open(line: &str) -> Option<(String, Option<String>)> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with("```") {
        return None;
    }
    let ticks: String = trimmed.chars().take_while(|c| *c == '`').collect();
    if ticks.len() < 3 {
        return None;
    }
    let lang = trimmed[ticks.len()..].trim();
    let language = (!lang.is_empty()).then(|| lang.to_string());
    Some((ticks, language))
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
    use crate::config::{
        FrecencyConfig, FuzzyWeights, LintRuleConfig, SearchConfig, Theme, UiConfig,
    };
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
                xdg_snippets_dir: root.clone(),
                snippet_overrides_active: false,
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
            lint: BTreeMap::new(),
            suggestion_commands: SuggestionCommandsConfig {
                timeout_ms: 50,
                allow_commands: true,
            },
        }
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
    fn text_only_sections_emit_dedicated_lint() {
        let root = temp_dir("text-only");
        fs::write(
            root.join("snippets.md"),
            "## Example only\n\n```text\nnot executable\n```\n",
        )
        .unwrap();
        let mut out = Vec::new();
        let result = run(
            &config(root),
            LintOptions {
                strict: true,
                json: false,
            },
            &mut out,
        )
        .unwrap();

        assert!(!result.findings.iter().any(|finding| {
            finding.code == CODE_MISSING_CODE_LANGUAGE || finding.code == CODE_MARKDOWN_STRUCTURE
        }));
        assert!(
            result
                .findings
                .iter()
                .any(|finding| finding.code == CODE_TEXT_ONLY_SECTION),
            "expected CODE_TEXT_ONLY_SECTION finding, got: {:?}",
            result.findings
        );
    }

    #[test]
    fn text_then_executable_section_has_no_structure_findings() {
        let root = temp_dir("text-then-exec");
        fs::write(
            root.join("snippets.md"),
            "## Demo\n\n```text\nexample\n```\n\n```bash\necho hi\n```\n",
        )
        .unwrap();
        let mut out = Vec::new();
        let result = run(
            &config(root),
            LintOptions {
                strict: true,
                json: false,
            },
            &mut out,
        )
        .unwrap();

        assert!(!result.findings.iter().any(|finding| {
            finding.code == CODE_TEXT_ONLY_SECTION
                || finding.code == CODE_MARKDOWN_STRUCTURE
                || finding.code == CODE_MISSING_CODE_LANGUAGE
        }));
    }

    #[test]
    fn unclosed_text_fence_is_flagged_as_unclosed() {
        let root = temp_dir("unclosed-text");
        fs::write(
            root.join("snippets.md"),
            "## Demo\n\n```text\nno close here\n",
        )
        .unwrap();
        let mut out = Vec::new();
        let result = run(
            &config(root),
            LintOptions {
                strict: true,
                json: false,
            },
            &mut out,
        )
        .unwrap();

        assert!(
            result.findings.iter().any(|finding| {
                finding.code == CODE_MARKDOWN_STRUCTURE
                    && finding.message.contains("code fence is not closed")
            }),
            "expected unclosed-fence finding, got: {:?}",
            result.findings
        );
    }

    #[test]
    fn lint_config_can_ignore_command_glob() {
        let root = temp_dir("ignore-command");
        fs::write(
            root.join("snippets.md"),
            "## Demo\n\n```bash\necho <@file:rg --files '*.nope'>\n```\n",
        )
        .unwrap();
        let mut cfg = config(root);
        cfg.lint.insert(
            "suggestion-command-failed".to_string(),
            LintRuleConfig {
                ignore_command: vec!["*rg*".to_string()],
                ..LintRuleConfig::default()
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
        assert!(
            !result
                .findings
                .iter()
                .any(|finding| finding.code == CODE_SUGGESTION_COMMAND_FAILED)
        );
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

#[cfg(test)]
mod dependent_tests {
    use super::*;
    use crate::config::{FrecencyConfig, FuzzyWeights, SearchConfig, Theme, UiConfig};
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir(prefix: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "pb-lint-dep-{prefix}-{}-{}",
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
                xdg_snippets_dir: root.clone(),
                snippet_overrides_active: false,
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
            lint: BTreeMap::new(),
            suggestion_commands: SuggestionCommandsConfig {
                timeout_ms: 50,
                allow_commands: true,
            },
        }
    }

    fn lint(root: PathBuf) -> LintResult {
        let mut out = Vec::new();
        run(
            &config(root),
            LintOptions {
                strict: false,
                json: false,
            },
            &mut out,
        )
        .unwrap()
    }

    #[test]
    fn dependent_command_is_skipped_not_executed() {
        let root = temp_dir("skip");
        fs::write(
            root.join("snippets.md"),
            "## Demo\n\n```bash\n<@bucket> <@key:ls <#bucket>>\n```\n",
        )
        .unwrap();
        let result = lint(root);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == CODE_DEPENDENT_SUGGESTION_SKIPPED),
            "expected dependent-suggestion-skipped, got {:?}",
            result.findings
        );
        // Should NOT report a command failure for the dependent command.
        assert!(
            !result
                .findings
                .iter()
                .any(|f| f.code == CODE_SUGGESTION_COMMAND_FAILED)
        );
    }

    #[test]
    fn non_dependent_command_still_executes() {
        let root = temp_dir("nondep");
        fs::write(
            root.join("snippets.md"),
            "## Demo\n\n```bash\n<@x:echo a>\n```\n",
        )
        .unwrap();
        let result = lint(root);
        assert!(
            !result
                .findings
                .iter()
                .any(|f| f.code == CODE_DEPENDENT_SUGGESTION_SKIPPED)
        );
    }

    #[test]
    fn unknown_dependent_reference_is_flagged() {
        let root = temp_dir("unknown");
        fs::write(
            root.join("snippets.md"),
            "## Demo\n\n```bash\n<@x:echo <#nope>>\n```\n",
        )
        .unwrap();
        let result = lint(root);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == CODE_UNKNOWN_VARIABLE_REFERENCE),
            "got {:?}",
            result.findings
        );
    }

    #[test]
    fn forward_dependent_reference_is_flagged() {
        let root = temp_dir("forward");
        // Order: a, b. a's command references b (which comes after) → forward.
        fs::write(
            root.join("snippets.md"),
            "## Demo\n\n```bash\n<@a:echo <#b>> <@b:?x>\n```\n",
        )
        .unwrap();
        let result = lint(root);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == CODE_FORWARD_VARIABLE_REFERENCE),
            "got {:?}",
            result.findings
        );
    }

    #[test]
    fn self_dependent_reference_is_flagged() {
        let root = temp_dir("self");
        fs::write(
            root.join("snippets.md"),
            "## Demo\n\n```bash\n<@a:echo <#a>>\n```\n",
        )
        .unwrap();
        let result = lint(root);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == CODE_SELF_VARIABLE_REFERENCE),
            "got {:?}",
            result.findings
        );
    }

    #[test]
    fn escaped_dependent_reference_is_not_flagged() {
        let root = temp_dir("escaped");
        fs::write(
            root.join("snippets.md"),
            "## Demo\n\n```bash\n<@x:echo \\<#nope>>\n```\n",
        )
        .unwrap();
        let result = lint(root);
        assert!(
            !result
                .findings
                .iter()
                .any(|f| f.code == CODE_UNKNOWN_VARIABLE_REFERENCE
                    || f.code == CODE_DEPENDENT_SUGGESTION_SKIPPED),
            "escaped literal should not be reported: {:?}",
            result.findings
        );
    }

    #[test]
    fn ignore_command_suppresses_dependent_skip() {
        let root = temp_dir("ignore-cmd");
        fs::write(
            root.join("snippets.md"),
            "## Demo\n\n```bash\n<@bucket> <@key:ls <#bucket>>\n```\n",
        )
        .unwrap();
        let mut cfg = config(root);
        cfg.lint.insert(
            "dependent-suggestion-skipped".to_string(),
            crate::config::LintRuleConfig {
                ignore_command: vec!["*ls*".to_string()],
                ..Default::default()
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
        assert!(
            !result
                .findings
                .iter()
                .any(|f| f.code == CODE_DEPENDENT_SUGGESTION_SKIPPED),
            "suppression should hide finding, got: {:?}",
            result.findings
        );
    }
}
