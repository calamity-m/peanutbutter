//! Read-only snippet linting.

mod commands;
mod dependent_refs;
mod frontmatter;
mod gc;
mod output;
mod structure;
mod suppression;

use crate::config::AppConfig;
use crate::{discovery, parser};
use serde::Serialize;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

/// Broken or unterminated file frontmatter.
pub const CODE_BROKEN_FRONTMATTER: &str = "lint/broken-frontmatter";
/// Frontmatter or config variable definition is not referenced by snippets.
pub const CODE_UNUSED_VARIABLE: &str = "lint/unused-variable";
/// Two snippets in the same file slugify to the same base slug.
pub const CODE_DUPLICATE_SLUG: &str = "lint/duplicate-slug";
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
/// `<#name>` references a variable that is not declared in this snippet or
/// in its frontmatter.
pub const CODE_UNKNOWN_VARIABLE_REFERENCE: &str = "lint/unknown-variable-reference";
/// `<#name>` references a variable that appears later in prompt order.
pub const CODE_FORWARD_VARIABLE_REFERENCE: &str = "lint/forward-variable-reference";
/// `<#name>` references the variable whose own command it sits inside.
pub const CODE_SELF_VARIABLE_REFERENCE: &str = "lint/self-variable-reference";
/// A suggestion command or default's `<#...>` syntax could not be parsed.
pub const CODE_INVALID_DEPENDENT_REFERENCE: &str = "lint/invalid-dependent-reference";
/// Raw default splice references an upstream free-form value.
pub const CODE_RAW_DEFAULT_UNTRUSTED_UPSTREAM: &str = "lint/raw-default-untrusted-upstream";
/// The same variable name and command appear inline in multiple snippets in the same file.
pub const CODE_DUPLICATE_INLINE_COMMAND: &str = "lint/duplicate-inline-command";

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

/// Per-file state threaded through all lint rule functions.
pub(super) struct FileContext {
    path: PathBuf,
    content: String,
    parsed: crate::domain::SnippetFile,
}

/// Run lint over the configured snippet roots and write the selected output
/// format to `writer`. This function is read-only: suggestion commands are not
/// executed, and frecency GC is queried without saving or mutating state.
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
                path,
                content,
                parsed,
            });
        }
    }

    for file in &files {
        findings.extend(frontmatter::lint_frontmatter_source(
            &file.path,
            &file.content,
        ));
        findings.extend(structure::lint_duplicate_slugs(file));
        findings.extend(frontmatter::lint_unused_file_variables(file));
        findings.extend(commands::lint_static_inline_commands(file));
        findings.extend(commands::lint_duplicate_inline_commands(file));
        findings.extend(dependent_refs::lint_dependent_references(
            file,
            &config.variables,
        ));
        if options.strict {
            findings.extend(structure::lint_markdown_structure(
                &file.path,
                &file.content,
            ));
            findings.extend(structure::lint_missing_code_languages(file));
            findings.extend(frontmatter::lint_frontmatter_overrides(
                file,
                &config.variables,
            ));
        }
    }

    findings.extend(frontmatter::lint_unused_config_variables(
        &config.variables,
        &config.paths.config_file,
        &files,
    ));
    let index =
        crate::index::SnippetIndex::from_files(files.iter().map(|file| file.parsed.clone()));
    findings.extend(gc::lint_gc(&config.paths, &index)?);
    suppression::attach_suppress_paths(&mut findings, &files);
    findings.retain(|finding| !suppression::is_suppressed(finding, &config.lint));
    sort_findings(&mut findings);
    let result = LintResult { findings };
    if options.json {
        output::render_json(&result, writer)?;
    } else {
        output::render_pretty(&result, writer)?;
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
        path: path.to_path_buf(),
        content: content.to_string(),
        parsed,
    };
    let mut findings = Vec::new();
    findings.extend(frontmatter::lint_frontmatter_source(path, content));
    findings.extend(structure::lint_duplicate_slugs(&ctx));
    findings.extend(frontmatter::lint_unused_file_variables(&ctx));
    findings.extend(commands::lint_static_inline_commands(&ctx));
    findings.extend(commands::lint_duplicate_inline_commands(&ctx));
    findings.extend(dependent_refs::lint_dependent_references(
        &ctx,
        &config.variables,
    ));
    findings.extend(structure::lint_markdown_structure(path, content));
    findings.extend(structure::lint_missing_code_languages(&ctx));
    findings.extend(frontmatter::lint_frontmatter_overrides(
        &ctx,
        &config.variables,
    ));
    suppression::attach_suppress_paths(&mut findings, &[ctx]);
    findings.retain(|f| !suppression::is_suppressed(f, &config.lint));
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

/// Construct a [`LintFinding`] with no span or suppress fields set.
pub(super) fn finding(
    severity: LintSeverity,
    code: &'static str,
    path: PathBuf,
    line: Option<usize>,
    snippet_id: Option<crate::domain::SnippetId>,
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

fn sort_findings(findings: &mut [LintFinding]) {
    findings.sort_by(|a, b| {
        a.path
            .cmp(&b.path)
            .then(a.line.cmp(&b.line))
            .then(a.code.cmp(b.code))
            .then(a.message.cmp(&b.message))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        FrecencyConfig, FuzzyWeights, LintRuleConfig, Paths, SearchConfig,
        SuggestionCommandsConfig, Theme, UiConfig,
    };
    use std::collections::BTreeMap;
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
            keybinds: Default::default(),
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
            crate::domain::VariableSpec {
                suggestions: vec!["a".to_string()],
                ..Default::default()
            },
        );
        cfg.variables.insert(
            "unused".to_string(),
            crate::domain::VariableSpec {
                suggestions: vec!["b".to_string()],
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
            "---\nvariables:\n  file:\n    command: rg <#\n---\n\n## Demo\n\n```bash\necho <@file>\n```\n",
        )
        .unwrap();
        let mut cfg = config(root);
        cfg.lint.insert(
            "invalid-dependent-reference".to_string(),
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
                .any(|finding| finding.code == CODE_INVALID_DEPENDENT_REFERENCE)
        );
    }

    #[test]
    fn duplicate_inline_command_across_snippets_is_reported() {
        let root = temp_dir("dup-inline-cmd");
        fs::write(
            root.join("snippets.md"),
            "## First\n\n```bash\necho <@env:kubectl get ns>\n```\n\n\
             ## Second\n\n```bash\necho <@env:kubectl get ns>\n```\n",
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
        let finding = result
            .findings
            .iter()
            .find(|f| f.code == CODE_DUPLICATE_INLINE_COMMAND)
            .expect("expected duplicate-inline-command finding");
        assert!(finding.message.contains("env"));
    }

    #[test]
    fn duplicate_inline_command_different_names_not_reported() {
        let root = temp_dir("dup-inline-cmd-diff");
        fs::write(
            root.join("snippets.md"),
            "## First\n\n```bash\necho <@a:ls>\n```\n\n\
             ## Second\n\n```bash\necho <@b:ls>\n```\n",
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
        assert!(
            !result
                .findings
                .iter()
                .any(|f| f.code == CODE_DUPLICATE_INLINE_COMMAND)
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
    use crate::config::{
        FrecencyConfig, FuzzyWeights, Paths, SearchConfig, SuggestionCommandsConfig, Theme,
        UiConfig,
    };
    use std::collections::BTreeMap;
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
            keybinds: Default::default(),
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
    fn dependent_command_is_valid_syntax() {
        let root = temp_dir("dependent-syntax");
        fs::write(
            root.join("snippets.md"),
            "## Demo\n\n```bash\n<@bucket> <@key:ls <#bucket>>\n```\n",
        )
        .unwrap();
        let result = lint(root);
        assert!(result.findings.is_empty(), "got {:?}", result.findings);
    }

    #[test]
    fn invalid_command_reference_syntax_is_flagged() {
        let root = temp_dir("invalid-command-ref");
        fs::write(
            root.join("snippets.md"),
            "---\nvariables:\n  key:\n    command: ls <#\n---\n\n## Demo\n\n```bash\n<@key>\n```\n",
        )
        .unwrap();
        let result = lint(root);
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == CODE_INVALID_DEPENDENT_REFERENCE),
            "got {:?}",
            result.findings
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
                .any(|f| f.code == CODE_UNKNOWN_VARIABLE_REFERENCE),
            "escaped literal should not be reported: {:?}",
            result.findings
        );
    }

    #[test]
    fn unknown_dependent_default_reference_is_flagged() {
        let root = temp_dir("default-unknown");
        fs::write(
            root.join("snippets.md"),
            "## Demo\n\n```bash\n<@b:?<#nope>>\n```\n",
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
    fn forward_and_self_dependent_default_references_are_flagged() {
        let root = temp_dir("default-order");
        fs::write(
            root.join("snippets.md"),
            "## Demo\n\n```bash\n<@a:?<#b>> <@b> <@c:?<#c>>\n```\n",
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
    fn escaped_default_reference_is_not_flagged_or_skipped() {
        let root = temp_dir("default-escaped");
        fs::write(
            root.join("snippets.md"),
            "## Demo\n\n```bash\n<@b:?\\<#nope>>\n```\n",
        )
        .unwrap();
        let result = lint(root);
        assert!(
            !result
                .findings
                .iter()
                .any(|f| f.code == CODE_UNKNOWN_VARIABLE_REFERENCE),
            "got {:?}",
            result.findings
        );
    }

    #[test]
    fn raw_default_untrusted_upstream_warns_only_for_free_form_raw() {
        let root = temp_dir("raw-default");
        fs::write(
            root.join("snippets.md"),
            "## Demo\n\n```bash\n<@a> <@b:?<#a:raw>> <@c:?host> <@d:?<#c:raw>> <@e:?<#a>>\n```\n",
        )
        .unwrap();
        let result = lint(root);
        let warnings: Vec<_> = result
            .findings
            .iter()
            .filter(|f| f.code == CODE_RAW_DEFAULT_UNTRUSTED_UPSTREAM)
            .collect();
        assert_eq!(warnings.len(), 1, "got {:?}", result.findings);
    }

    #[test]
    fn hint_placeholder_is_not_a_command_and_stays_free_form_upstream() {
        let root = temp_dir("hint-placeholder");
        // `<@a:@echo hi>` is a hint, so it must not trip the inline-command
        // lints, but it is still an unconstrained upstream for `:raw` splices.
        fs::write(
            root.join("snippets.md"),
            "## Demo\n\n```bash\n<@a:@echo hi> <@b:?<#a:raw>>\n```\n",
        )
        .unwrap();
        let result = lint(root);
        assert!(
            !result
                .findings
                .iter()
                .any(|f| f.code == CODE_STATIC_INLINE_COMMAND
                    || f.code == CODE_INVALID_DEPENDENT_REFERENCE),
            "got {:?}",
            result.findings
        );
        assert!(
            result
                .findings
                .iter()
                .any(|f| f.code == CODE_RAW_DEFAULT_UNTRUSTED_UPSTREAM),
            "got {:?}",
            result.findings
        );
    }
}
