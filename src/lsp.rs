//! Language Server Protocol server for peanutbutter snippet files.
//!
//! Starts an LSP server over stdio (`peanutbutter lsp`) that provides:
//! - **Diagnostics** — lint findings published on open/change
//! - **Completions** — frontmatter keys and `<@variable>` placeholders
//! - **Hover** — variable spec on `<@name>` and docs on frontmatter keys
//! - **Go-to-definition** — `<@name>` in a code block → `variables.name:` in frontmatter
//!
//! # Activation scope
//!
//! The server only activates for `.md` files that sit under a directory tree
//! containing one of the marker files listed in [`MARKER_FILENAMES`]. The
//! marker directory becomes the snippet root used for linting. Files outside
//! any marked tree receive empty diagnostics and no completions/hover/definitions.

use crate::config::{self, AppConfig};
use crate::lint;
use crate::parser;
use serde::Deserialize;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

mod code_actions;
mod completions;
mod hover;
mod navigation;
mod position;
mod semantic_tokens;

/// Run the LSP server over stdio until the client disconnects.
pub fn run_lsp_server() {
    let config = match config::load() {
        Ok(c) => c,
        Err(err) => {
            eprintln!("peanutbutter-lsp: failed to load config: {err}");
            return;
        }
    };
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async {
        let stdin = tokio::io::stdin();
        let stdout = tokio::io::stdout();
        let (service, socket) = LspService::new(|client| Backend {
            client,
            documents: RwLock::new(HashMap::new()),
            config: Arc::new(config),
        });
        Server::new(stdin, stdout, socket).serve(service).await;
    });
}

// ---------------------------------------------------------------------------
// Internal state
// ---------------------------------------------------------------------------

struct Backend {
    client: Client,
    /// In-memory document store: URI → current text content.
    documents: RwLock<HashMap<Url, String>>,
    config: Arc<AppConfig>,
}

// ---------------------------------------------------------------------------
// LSP implementation
// ---------------------------------------------------------------------------

fn server_capabilities() -> ServerCapabilities {
    ServerCapabilities {
        // LSP columns are UTF-16 code units. Advertising the default explicitly
        // keeps every handler on the one encoding converted by `position`.
        position_encoding: Some(PositionEncodingKind::UTF16),
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![
                "<".to_string(),
                "@".to_string(),
                " ".to_string(),
                ":".to_string(),
            ]),
            ..Default::default()
        }),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        definition_provider: Some(OneOf::Left(true)),
        references_provider: Some(OneOf::Left(true)),
        code_action_provider: Some(CodeActionProviderCapability::Simple(true)),
        semantic_tokens_provider: Some(SemanticTokensServerCapabilities::SemanticTokensOptions(
            SemanticTokensOptions {
                legend: SemanticTokensLegend {
                    token_types: semantic_tokens::TOKEN_TYPES.to_vec(),
                    token_modifiers: vec![],
                },
                full: Some(SemanticTokensFullOptions::Bool(true)),
                range: Some(false),
                ..Default::default()
            },
        )),
        ..Default::default()
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: server_capabilities(),
            server_info: Some(ServerInfo {
                name: "peanutbutter-lsp".to_string(),
                version: Some(env!("CARGO_PKG_VERSION").to_string()),
            }),
        })
    }

    async fn initialized(&self, _: InitializedParams) {
        self.client
            .log_message(MessageType::INFO, "peanutbutter LSP ready")
            .await;
    }

    async fn shutdown(&self) -> Result<()> {
        Ok(())
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let uri = params.text_document.uri;
        let text = params.text_document.text;
        self.documents
            .write()
            .await
            .insert(uri.clone(), text.clone());
        self.publish_diagnostics(&uri, &text).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        // Full sync: last content_change is the full document.
        if let Some(change) = params.content_changes.into_iter().last() {
            self.documents
                .write()
                .await
                .insert(uri.clone(), change.text.clone());
            self.publish_diagnostics(&uri, &change.text).await;
        }
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        let uri = params.text_document.uri;
        self.documents.write().await.remove(&uri);
        // Clear diagnostics when the file is closed.
        self.client.publish_diagnostics(uri, vec![], None).await;
    }

    async fn completion(&self, params: CompletionParams) -> Result<Option<CompletionResponse>> {
        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        if find_lsp_workspace(&uri_to_path(uri)).is_none() {
            return Ok(None);
        }
        let docs = self.documents.read().await;
        let Some(content) = docs.get(uri) else {
            return Ok(None);
        };
        Ok(completions::compute_completions(
            content,
            pos,
            &self.config.variables,
        ))
    }

    async fn hover(&self, params: HoverParams) -> Result<Option<Hover>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        if find_lsp_workspace(&uri_to_path(uri)).is_none() {
            return Ok(None);
        }
        let docs = self.documents.read().await;
        let Some(content) = docs.get(uri) else {
            return Ok(None);
        };
        Ok(hover::compute_hover(content, pos, &self.config.variables))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> Result<Option<GotoDefinitionResponse>> {
        let uri = &params.text_document_position_params.text_document.uri;
        let pos = params.text_document_position_params.position;
        if find_lsp_workspace(&uri_to_path(uri)).is_none() {
            return Ok(None);
        }
        let docs = self.documents.read().await;
        let Some(content) = docs.get(uri) else {
            return Ok(None);
        };
        Ok(navigation::compute_definition(
            uri,
            content,
            pos,
            self.config.as_ref(),
        ))
    }

    async fn references(&self, params: ReferenceParams) -> Result<Option<Vec<Location>>> {
        let uri = &params.text_document_position.text_document.uri;
        let pos = params.text_document_position.position;
        if find_lsp_workspace(&uri_to_path(uri)).is_none() {
            return Ok(None);
        }
        let docs = self.documents.read().await;
        let Some(content) = docs.get(uri) else {
            return Ok(None);
        };
        Ok(navigation::compute_references(uri, content, pos))
    }

    async fn code_action(&self, params: CodeActionParams) -> Result<Option<CodeActionResponse>> {
        let uri = &params.text_document.uri;
        if find_lsp_workspace(&uri_to_path(uri)).is_none() {
            return Ok(None);
        }
        let docs = self.documents.read().await;
        let Some(content) = docs.get(uri) else {
            return Ok(None);
        };
        Ok(code_actions::compute_code_actions_filtered(
            uri,
            content,
            params.range,
            &self.config.variables,
            params.context.only.as_deref(),
        ))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = &params.text_document.uri;
        if find_lsp_workspace(&uri_to_path(uri)).is_none() {
            return Ok(None);
        }
        let docs = self.documents.read().await;
        let Some(content) = docs.get(uri) else {
            return Ok(None);
        };
        Ok(semantic_tokens::compute_semantic_tokens(content))
    }
}

// ---------------------------------------------------------------------------
// Diagnostics
// ---------------------------------------------------------------------------

impl Backend {
    async fn publish_diagnostics(&self, uri: &Url, content: &str) {
        let path = uri_to_path(uri);
        let Some(workspace) = find_lsp_workspace(&path) else {
            // Outside a peanutbutter snippet tree; clear any stale diagnostics.
            self.client
                .publish_diagnostics(uri.clone(), vec![], None)
                .await;
            return;
        };
        let effective_config;
        let config = if workspace.config.skip_rules.is_empty() {
            self.config.as_ref()
        } else {
            effective_config =
                config_with_skipped_rules(&self.config, &workspace.config.skip_rules);
            &effective_config
        };
        let findings = lint::lint_file(&path, &workspace.root, content, config);
        let diagnostics = findings
            .iter()
            .map(|f| lint_finding_to_diagnostic(f, content))
            .collect();
        self.client
            .publish_diagnostics(uri.clone(), diagnostics, None)
            .await;
    }
}

/// Convert a document URI to an absolute filesystem path.
fn uri_to_path(uri: &Url) -> PathBuf {
    uri.to_file_path()
        .unwrap_or_else(|_| PathBuf::from(uri.path()))
}

/// Convert a [`LintFinding`] to an LSP [`Diagnostic`].
///
/// Line numbers in findings are 1-based; LSP positions are 0-based.
fn lint_finding_to_diagnostic(finding: &lint::LintFinding, content: &str) -> Diagnostic {
    let line = finding.line.unwrap_or(1).saturating_sub(1) as u32;
    let source_line = content.lines().nth(line as usize).unwrap_or("");
    let range = match (finding.col_start, finding.col_end) {
        (Some(c0), Some(c1)) => position::byte_span_to_range(line, source_line, c0, c1),
        _ => position::byte_span_to_range(line, source_line, 0, source_line.len()),
    };
    let severity = match finding.severity {
        lint::LintSeverity::Error => DiagnosticSeverity::ERROR,
        lint::LintSeverity::Warning => DiagnosticSeverity::WARNING,
    };
    let message = match &finding.detail {
        Some(detail) => format!("{}\n{detail}", finding.message),
        None => finding.message.clone(),
    };
    Diagnostic {
        range,
        severity: Some(severity),
        code: Some(NumberOrString::String(finding.code.to_string())),
        source: Some("peanutbutter".to_string()),
        message,
        ..Default::default()
    }
}

/// Find the 0-based line index of `  <name>:` inside the `variables:` frontmatter block.
fn find_variable_declaration_line(lines: &[&str], name: &str) -> Option<usize> {
    let fm_end = frontmatter_end_line(lines)?;
    let mut in_variables = false;
    for (i, line) in lines[1..fm_end].iter().enumerate() {
        let actual_line = i + 1;
        let trimmed = line.trim_start();
        if trimmed == "variables:" {
            in_variables = true;
            continue;
        }
        if in_variables {
            let indent = line.len() - trimmed.len();
            if indent == 0 {
                // Back to top-level key, variables block ended.
                break;
            }
            if indent >= 2 {
                // Could be `  name:` — check for variable name at exactly 2-space indent.
                let key = trimmed.split(':').next().unwrap_or("").trim();
                if key == name {
                    return Some(actual_line);
                }
            }
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

pub(super) fn enclosing_placeholder_owner(before_cursor: &str, hash_pos: usize) -> Option<String> {
    let at_pos = before_cursor[..hash_pos].rfind("<@")?;
    let inner = &before_cursor[at_pos + 2..];
    let name = inner.split(':').next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

pub(super) fn snippet_at_line(content: &str, line_idx: usize) -> Option<crate::domain::Snippet> {
    let parsed = parser::parse_file(std::path::Path::new(""), std::path::Path::new(""), content);
    let ranges = parser::snippet_line_ranges(std::path::Path::new(""), content);
    let range = ranges
        .iter()
        .find(|range| line_idx >= range.start_line && line_idx < range.end_line)?;
    parsed
        .snippets
        .into_iter()
        .find(|snippet| snippet.id == range.id)
}

/// Marker file names that identify a directory tree as a peanutbutter snippet root.
///
/// The server only activates for `.md` files that have one of these files in
/// an ancestor directory. Any of the three names is accepted so teams can pick
/// a convention that fits their project layout.
pub const MARKER_FILENAMES: &[&str] = &[
    ".peanutbutter.toml",
    "peanutbutter.toml",
    "_peanutbutter.toml",
];

/// Per-workspace LSP settings loaded from a marker file.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default)]
struct LspWorkspaceConfig {
    /// Glob patterns for workspace-relative files or directories the LSP ignores.
    #[serde(deserialize_with = "config::string_or_vec")]
    ignore: Vec<String>,
    /// Glob patterns that opt files in; an empty list allows every non-ignored file.
    #[serde(deserialize_with = "config::string_or_vec")]
    attach_only: Vec<String>,
    /// Lint rule names disabled for this workspace, with or without `lint/`.
    #[serde(deserialize_with = "config::string_or_vec")]
    skip_rules: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LspWorkspace {
    root: PathBuf,
    config: LspWorkspaceConfig,
}

fn find_lsp_workspace(path: &Path) -> Option<LspWorkspace> {
    if path.extension().and_then(|extension| extension.to_str()) != Some("md") {
        return None;
    }
    let (root, marker_path) = find_marker(path)?;
    let config = parse_marker_config(&marker_path).ok()?;
    if !workspace_allows_path(&root, path, &config) {
        return None;
    }
    Some(LspWorkspace { root, config })
}

fn find_marker(path: &Path) -> Option<(PathBuf, PathBuf)> {
    let mut dir = path.parent()?;
    loop {
        for name in MARKER_FILENAMES {
            let marker_path = dir.join(name);
            if marker_path.exists() {
                return Some((dir.to_path_buf(), marker_path));
            }
        }
        dir = dir.parent()?;
    }
}

fn parse_marker_config(path: &Path) -> std::result::Result<LspWorkspaceConfig, String> {
    let raw = fs::read_to_string(path).map_err(|err| err.to_string())?;
    toml::from_str(&raw).map_err(|err| err.to_string())
}

fn workspace_allows_path(root: &Path, path: &Path, config: &LspWorkspaceConfig) -> bool {
    let relative = workspace_relative_path(root, path);
    !glob_list_matches_path(&config.ignore, &relative)
        && (config.attach_only.is_empty() || glob_list_matches_path(&config.attach_only, &relative))
}

fn workspace_relative_path(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn glob_list_matches_path(patterns: &[String], relative_path: &str) -> bool {
    patterns
        .iter()
        .any(|pattern| glob_matches_path(pattern, relative_path))
}

fn glob_matches_path(pattern: &str, relative_path: &str) -> bool {
    let pattern = pattern.trim().trim_matches('/');
    if pattern.is_empty() {
        return false;
    }
    glob_matches_segments(
        &pattern.split('/').collect::<Vec<_>>(),
        &relative_path.split('/').collect::<Vec<_>>(),
    ) || (!contains_glob_meta(pattern) && relative_path.starts_with(&format!("{pattern}/")))
}

fn contains_glob_meta(pattern: &str) -> bool {
    pattern.as_bytes().iter().any(|b| matches!(b, b'*' | b'?'))
}

fn glob_matches_segments(pattern: &[&str], value: &[&str]) -> bool {
    match (pattern, value) {
        ([], []) => true,
        ([], _) => false,
        (["**", rest @ ..], _) => {
            glob_matches_segments(rest, value)
                || (!value.is_empty() && glob_matches_segments(pattern, &value[1..]))
        }
        ([segment_pattern, rest_pattern @ ..], [segment, rest_value @ ..]) => {
            glob_matches_segment(segment_pattern, segment)
                && glob_matches_segments(rest_pattern, rest_value)
        }
        _ => false,
    }
}

fn glob_matches_segment(pattern: &str, value: &str) -> bool {
    glob_matches_segment_bytes(pattern.as_bytes(), value.as_bytes())
}

fn glob_matches_segment_bytes(pattern: &[u8], value: &[u8]) -> bool {
    match (pattern, value) {
        ([], []) => true,
        ([], _) => false,
        ([b'*', rest @ ..], _) => {
            glob_matches_segment_bytes(rest, value)
                || (!value.is_empty() && glob_matches_segment_bytes(pattern, &value[1..]))
        }
        ([b'?', rest @ ..], [_, value_rest @ ..]) => glob_matches_segment_bytes(rest, value_rest),
        ([p, rest @ ..], [v, value_rest @ ..]) if p == v => {
            glob_matches_segment_bytes(rest, value_rest)
        }
        _ => false,
    }
}

fn config_with_skipped_rules(config: &AppConfig, skip_rules: &[String]) -> AppConfig {
    let mut config = config.clone();
    for rule in skip_rules {
        let rule = rule.trim().strip_prefix("lint/").unwrap_or(rule.trim());
        if rule.is_empty() {
            continue;
        }
        config.lint.entry(rule.to_string()).or_default().disable = true;
    }
    config
}

/// Return the 0-based line index of the closing `---` of the frontmatter block,
/// or `None` if there is no valid frontmatter.
pub(super) fn frontmatter_end_line(lines: &[&str]) -> Option<usize> {
    if lines.first().map(|l| l.trim()) != Some("---") {
        return None;
    }
    lines[1..]
        .iter()
        .position(|l| l.trim() == "---")
        .map(|i| i + 1)
}

/// Extract the variable name and byte span `[start, end)` of a `<#name…>` or
/// `<#name:raw>` dependent reference that the cursor sits within. Returns
/// the bool `raw` indicating which form was used.
pub(super) fn dependent_ref_at(
    line: &str,
    char_idx: usize,
) -> Option<(String, usize, usize, bool)> {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len().saturating_sub(1) {
        if bytes[i] == b'<' && bytes[i + 1] == b'#' {
            let start = i;
            let (end, inner_end) = match bytes[start..].iter().position(|&b| b == b'>') {
                Some(rel) => (start + rel + 1, start + rel),
                None => (bytes.len(), bytes.len()),
            };
            if char_idx >= start && char_idx <= end {
                let inner = &line[start + 2..inner_end];
                let (name_part, raw) = match inner.split_once(':') {
                    Some((n, m)) => (n, m == "raw"),
                    None => (inner, false),
                };
                let name = name_part.trim().to_string();
                if name.is_empty() {
                    return None;
                }
                return Some((name, start, end, raw));
            }
            i = end;
        } else {
            i += 1;
        }
    }
    None
}

/// Extract the variable name and byte span `[start, end)` of a `<@name…>` or
/// `<@name:…>` placeholder that the cursor (at `char_idx`) sits within.
pub(super) fn placeholder_at(line: &str, char_idx: usize) -> Option<(String, usize, usize)> {
    // Scan for `<@…>` spans overlapping the cursor position.
    let bytes = line.as_bytes();
    let mut i = 0;
    while i < bytes.len().saturating_sub(1) {
        if bytes[i] == b'<' && bytes[i + 1] == b'@' {
            let start = i;
            let (end, inner_end) = match bytes[start..].iter().position(|&b| b == b'>') {
                Some(rel) => (start + rel + 1, start + rel),
                None => (bytes.len(), bytes.len()),
            };
            if char_idx >= start && char_idx <= end {
                // Extract name: everything between `<@` and the first `:` or `>`
                let inner = &line[start + 2..inner_end];
                let name = inner.split(':').next().unwrap_or(inner).to_string();
                return Some((name, start, end));
            }
            i = end;
        } else {
            i += 1;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn tmp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "pb-lsp-test-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .subsec_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn empty_app_config() -> crate::config::AppConfig {
        use crate::config::{Paths, SearchConfig, SuggestionCommandsConfig, Theme, UiConfig};
        crate::config::AppConfig {
            paths: Paths {
                snippet_roots: vec![],
                xdg_snippets_dir: std::path::PathBuf::new(),
                snippet_overrides_active: false,
                state_file: std::path::PathBuf::new(),
                config_file: std::path::PathBuf::new(),
            },
            ui: UiConfig::default(),
            search: SearchConfig::default(),
            variables: std::collections::BTreeMap::new(),
            theme: Theme::default(),
            suggestion_commands: SuggestionCommandsConfig::default(),
            lint: std::collections::BTreeMap::new(),
            keybinds: Default::default(),
        }
    }

    #[test]
    fn find_marker_dot_peanutbutter_toml() {
        let root = tmp_dir();
        fs::write(root.join(".peanutbutter.toml"), "").unwrap();
        let file = root.join("sub").join("snippets.md");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "").unwrap();
        assert_eq!(find_marker(&file).map(|(root, _)| root), Some(root.clone()));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_marker_peanutbutter_toml() {
        let root = tmp_dir();
        fs::write(root.join("peanutbutter.toml"), "").unwrap();
        let file = root.join("snippets.md");
        fs::write(&file, "").unwrap();
        assert_eq!(find_marker(&file).map(|(root, _)| root), Some(root.clone()));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_marker_underscore_peanutbutter_toml() {
        let root = tmp_dir();
        fs::write(root.join("_peanutbutter.toml"), "").unwrap();
        let file = root.join("snippets.md");
        fs::write(&file, "").unwrap();
        assert_eq!(find_marker(&file).map(|(root, _)| root), Some(root.clone()));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_marker_returns_none_when_no_marker() {
        let root = tmp_dir();
        let file = root.join("snippets.md");
        fs::write(&file, "").unwrap();
        assert_eq!(find_marker(&file).map(|(root, _)| root), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_marker_nearest_ancestor_wins() {
        // Inner marker should win over outer one.
        let outer = tmp_dir();
        let inner = outer.join("sub");
        fs::create_dir_all(&inner).unwrap();
        fs::write(outer.join(".peanutbutter.toml"), "").unwrap();
        fs::write(inner.join("peanutbutter.toml"), "").unwrap();
        let file = inner.join("snippets.md");
        fs::write(&file, "").unwrap();
        assert_eq!(
            find_marker(&file).map(|(root, _)| root),
            Some(inner.clone())
        );
        fs::remove_dir_all(&outer).unwrap();
    }

    #[test]
    fn marker_config_deserializes_lsp_settings() {
        let root = tmp_dir();
        let marker = root.join(".peanutbutter.toml");
        fs::write(
            &marker,
            r#"
ignore = "archive/**"
attach_only = ["active/**"]
skip_rules = ["unused-variable", "lint/markdown-structure"]
"#,
        )
        .unwrap();

        let config = parse_marker_config(&marker).unwrap();

        assert_eq!(config.ignore, vec!["archive/**"]);
        assert_eq!(config.attach_only, vec!["active/**"]);
        assert_eq!(
            config.skip_rules,
            vec!["unused-variable", "lint/markdown-structure"]
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lsp_workspace_ignores_matching_files() {
        let root = tmp_dir();
        fs::write(
            root.join(".peanutbutter.toml"),
            "ignore = [\"vendor/**\"]\n",
        )
        .unwrap();
        let file = root.join("vendor").join("snippets.md");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "").unwrap();

        assert!(find_lsp_workspace(&file).is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lsp_workspace_rejects_invalid_marker_config() {
        let root = tmp_dir();
        fs::write(root.join(".peanutbutter.toml"), "ignore = [\n").unwrap();
        let file = root.join("snippets.md");
        fs::write(&file, "").unwrap();

        assert!(find_lsp_workspace(&file).is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lsp_workspace_rejects_non_markdown_files() {
        let root = tmp_dir();
        fs::write(root.join(".peanutbutter.toml"), "").unwrap();
        let file = root.join("snippets.txt");
        fs::write(&file, "").unwrap();

        assert!(find_lsp_workspace(&file).is_none());
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn lsp_workspace_globs_do_not_cross_slashes_without_double_star() {
        assert!(glob_matches_path("*.md", "snippets.md"));
        assert!(!glob_matches_path("*.md", "nested/snippets.md"));
        assert!(glob_matches_path("nested/**", "nested/deep/snippets.md"));
        assert!(!glob_matches_path("nested/*", "nested/deep/snippets.md"));
        assert!(glob_matches_path("generated", "generated/snippets.md"));
    }

    #[test]
    fn lsp_workspace_respects_attach_only() {
        let root = tmp_dir();
        fs::write(
            root.join(".peanutbutter.toml"),
            "attach_only = [\"snippets/**\"]\n",
        )
        .unwrap();
        let allowed = root.join("snippets").join("ok.md");
        let denied = root.join("notes").join("no.md");
        fs::create_dir_all(allowed.parent().unwrap()).unwrap();
        fs::create_dir_all(denied.parent().unwrap()).unwrap();
        fs::write(&allowed, "").unwrap();
        fs::write(&denied, "").unwrap();

        assert_eq!(find_lsp_workspace(&allowed).unwrap().root, root);
        assert!(find_lsp_workspace(&denied).is_none());
        fs::remove_dir_all(allowed.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn skip_rules_disable_lints_for_workspace() {
        let config =
            config_with_skipped_rules(&empty_app_config(), &["lint/unused-variable".to_string()]);

        assert!(config.lint.get("unused-variable").unwrap().disable);
    }

    #[test]
    fn skip_rules_filter_workspace_diagnostics() {
        let root = tmp_dir();
        let file = root.join("snippets.md");
        let content = r#"---
variables:
  unused:
    default: nope
---
## Demo

```bash
echo hi
```
"#;
        fs::write(&file, content).unwrap();
        let base_config = empty_app_config();
        let skipped_config =
            config_with_skipped_rules(&base_config, &["unused-variable".to_string()]);

        let base_findings = lint::lint_file(&file, &root, content, &base_config);
        let skipped_findings = lint::lint_file(&file, &root, content, &skipped_config);

        assert!(
            base_findings
                .iter()
                .any(|finding| finding.code == lint::CODE_UNUSED_VARIABLE)
        );
        assert!(
            !skipped_findings
                .iter()
                .any(|finding| finding.code == lint::CODE_UNUSED_VARIABLE)
        );
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn server_advertises_utf16_positions() {
        assert_eq!(
            server_capabilities().position_encoding,
            Some(PositionEncodingKind::UTF16)
        );
    }

    #[test]
    fn diagnostic_byte_spans_are_converted_to_utf16() {
        let content = "## Démo\n\n```bash\na\n```\n\n## Démo!\n\n```bash\nb\n```\n";
        let config = empty_app_config();
        let findings = lint::lint_file(Path::new("snippets.md"), Path::new("."), content, &config);
        let finding = findings
            .iter()
            .find(|finding| finding.code == lint::CODE_DUPLICATE_SLUG)
            .expect("duplicate slug finding");

        let diagnostic = lint_finding_to_diagnostic(finding, content);

        assert_eq!(diagnostic.range.start, Position::new(6, 3));
        assert_eq!(diagnostic.range.end, Position::new(6, 8));
    }
}

#[cfg(test)]
mod dependent_lsp_tests {
    use super::*;
    use crate::domain::VariableSpec;
    use std::collections::BTreeMap;

    use super::completions::compute_completions;
    use super::hover::compute_hover;
    use super::navigation::{compute_definition, compute_references};

    fn pos(line: u32, character: u32) -> Position {
        Position { line, character }
    }

    fn empty_config_vars() -> BTreeMap<String, VariableSpec> {
        BTreeMap::new()
    }

    fn empty_app_config() -> crate::config::AppConfig {
        use crate::config::{Paths, SearchConfig, SuggestionCommandsConfig, Theme, UiConfig};
        crate::config::AppConfig {
            paths: Paths {
                snippet_roots: vec![],
                xdg_snippets_dir: std::path::PathBuf::new(),
                snippet_overrides_active: false,
                state_file: std::path::PathBuf::new(),
                config_file: std::path::PathBuf::new(),
            },
            ui: UiConfig::default(),
            search: SearchConfig::default(),
            variables: BTreeMap::new(),
            theme: Theme::default(),
            suggestion_commands: SuggestionCommandsConfig::default(),
            lint: BTreeMap::new(),
            keybinds: Default::default(),
        }
    }

    #[test]
    fn dependent_ref_at_finds_token() {
        let line = "<@key:ls <#bucket>>";
        let (name, start, end, raw) = dependent_ref_at(line, 12).unwrap();
        assert_eq!(name, "bucket");
        assert_eq!(start, 9);
        assert_eq!(end, 18);
        assert!(!raw);
    }

    #[test]
    fn dependent_ref_at_handles_raw_modifier() {
        let line = "kubectl <#verb:raw> -o name";
        let (name, _, _, raw) = dependent_ref_at(line, 12).unwrap();
        assert_eq!(name, "verb");
        assert!(raw);
    }

    #[test]
    fn dependent_ref_at_returns_none_outside() {
        let line = "echo hi";
        assert!(dependent_ref_at(line, 2).is_none());
    }

    #[test]
    fn unterminated_tokens_ending_in_unicode_are_safe() {
        let dependent = "echo <#é";
        let placeholder = "echo <@é";

        assert_eq!(
            dependent_ref_at(dependent, dependent.len()).map(|(name, ..)| name),
            Some("é".to_string())
        );
        assert_eq!(
            placeholder_at(placeholder, placeholder.len()).map(|(name, ..)| name),
            Some("é".to_string())
        );
    }

    #[test]
    fn hover_on_dependent_ref_distinguishes_quoted_and_raw() {
        let content = "---\nvariables:\n  bucket:\n    suggestions: [a, b]\n---\n## D\n\n```bash\n<@bucket> <@key:ls <#bucket>>\n```\n";
        // Cursor is on `<#bucket>` inside the inline command. Body starts at
        // line 8 (0-based).
        let line_idx = 8;
        let line = content.lines().nth(line_idx).unwrap();
        let col = line.find("<#bucket").unwrap() + 2;
        let h = compute_hover(
            content,
            pos(line_idx as u32, col as u32),
            &empty_config_vars(),
        )
        .unwrap();
        let HoverContents::Markup(m) = h.contents else {
            panic!("expected markup");
        };
        assert!(m.value.contains("shell-quoted"), "got {}", m.value);
        assert!(m.value.contains("`<#bucket>`"));
    }

    #[test]
    fn definition_jumps_from_dependent_ref_to_frontmatter() {
        let content = "---\nvariables:\n  bucket:\n    suggestions: [a]\n---\n## D\n\n```bash\n<@bucket> <@key:ls <#bucket>>\n```\n";
        let uri = Url::parse("file:///x.md").unwrap();
        let line_idx = 8;
        let line = content.lines().nth(line_idx).unwrap();
        let col = line.find("<#bucket").unwrap() + 2;
        let def = compute_definition(
            &uri,
            content,
            pos(line_idx as u32, col as u32),
            &empty_app_config(),
        )
        .unwrap();
        let GotoDefinitionResponse::Scalar(loc) = def else {
            panic!("expected scalar");
        };
        // bucket: is declared on line 2 (0-based).
        assert_eq!(loc.range.start.line, 2);
    }

    #[test]
    fn references_includes_both_at_and_hash_uses() {
        let content = "---\nvariables:\n  bucket:\n    suggestions: [a]\n---\n## D\n\n```bash\n<@bucket> <@key:ls <#bucket>>\n```\n";
        let uri = Url::parse("file:///x.md").unwrap();
        // Cursor on the `<#bucket>` ref.
        let line_idx = 8;
        let line = content.lines().nth(line_idx).unwrap();
        let col = line.find("<#bucket").unwrap() + 2;
        let refs = compute_references(&uri, content, pos(line_idx as u32, col as u32)).unwrap();
        // Expect at least: one `<@bucket>` and one `<#bucket>` on the body line.
        let bodies: Vec<_> = refs.iter().filter(|l| l.range.start.line == 8).collect();
        assert!(bodies.len() >= 2, "got {refs:?}");
    }

    #[test]
    fn completion_after_hash_offers_only_variables_matching_prefix() {
        let content = "---\nvariables:\n  bucket:\n    suggestions: [a]\n  beach:\n    suggestions: [b]\n---\n## D\n\n```bash\n<@bucket> <@key:ls <#bu\n```\n";
        let line_idx = 10;
        let line = content.lines().nth(line_idx).unwrap();
        let col = line.len();
        let resp = compute_completions(
            content,
            pos(line_idx as u32, col as u32),
            &empty_config_vars(),
        )
        .unwrap();
        let CompletionResponse::Array(items) = resp else {
            panic!("expected array");
        };
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        // Prefix "bu" matches only `bucket`, not `beach`.
        assert!(labels.contains(&"bucket"), "got {labels:?}");
        assert!(!labels.contains(&"beach"), "got {labels:?}");
    }

    #[test]
    fn dependent_ref_at_finds_token_inside_default() {
        let line = "<@out:?<#bucket:raw>.out>";
        let (name, _, _, raw) = dependent_ref_at(line, line.find("bucket").unwrap()).unwrap();
        assert_eq!(name, "bucket");
        assert!(raw);
    }

    #[test]
    fn hover_definition_and_references_work_inside_default() {
        let content = "---\nvariables:\n  bucket:\n    suggestions: [a]\n---\n## D\n\n```bash\n<@bucket> <@out:?<#bucket:raw>.out>\n```\n";
        let uri = Url::parse("file:///x.md").unwrap();
        let line_idx = 8;
        let line = content.lines().nth(line_idx).unwrap();
        let col = line.find("<#bucket").unwrap() + 2;
        let hover = compute_hover(
            content,
            pos(line_idx as u32, col as u32),
            &empty_config_vars(),
        )
        .unwrap();
        let HoverContents::Markup(markup) = hover.contents else {
            panic!("expected markup");
        };
        assert!(markup.value.contains("raw splice"), "got {}", markup.value);

        let def = compute_definition(
            &uri,
            content,
            pos(line_idx as u32, col as u32),
            &empty_app_config(),
        )
        .unwrap();
        let GotoDefinitionResponse::Scalar(loc) = def else {
            panic!("expected scalar");
        };
        assert_eq!(loc.range.start.line, 2);

        let refs = compute_references(&uri, content, pos(line_idx as u32, col as u32)).unwrap();
        let bodies: Vec<_> = refs.iter().filter(|l| l.range.start.line == 8).collect();
        assert!(bodies.len() >= 2, "got {refs:?}");
    }

    #[test]
    fn default_dependent_completion_uses_earlier_prompt_order() {
        let content = "## D\n\n```bash\n<@a> <@out:?<#> <@later>\n```\n";
        let line_idx = 3;
        let line = content.lines().nth(line_idx).unwrap();
        let col = line.find("<#").unwrap() + 2;
        let resp = compute_completions(
            content,
            pos(line_idx as u32, col as u32),
            &empty_config_vars(),
        )
        .unwrap();
        let CompletionResponse::Array(items) = resp else {
            panic!("expected array");
        };
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"a"), "got {labels:?}");
        assert!(!labels.contains(&"out"), "got {labels:?}");
        assert!(!labels.contains(&"later"), "got {labels:?}");
    }

    #[test]
    fn completion_uses_utf16_columns_after_unicode() {
        let content =
            "---\nvariables:\n  name:\n    suggestions: [alice]\n---\n## D\n\n```bash\né <@\n```\n";
        let response =
            compute_completions(content, pos(8, 4), &empty_config_vars()).expect("completions");
        let CompletionResponse::Array(items) = response else {
            panic!("expected array");
        };

        assert!(items.iter().any(|item| item.label == "name"));
    }

    #[test]
    fn frontmatter_completion_does_not_offer_deprecated_hint() {
        let content = "---\nvariables:\n  input:\n    h\n---\n## D\n\n```bash\na\n```\n";
        let response =
            compute_completions(content, pos(3, 5), &empty_config_vars()).expect("completions");
        let CompletionResponse::Array(items) = response else {
            panic!("expected array");
        };

        assert!(!items.iter().any(|item| item.label == "hint"), "{items:?}");
    }

    #[test]
    fn hover_marks_existing_hint_specs_as_deprecated() {
        let content =
            "---\nvariables:\n  input:\n    hint: guidance\n---\n## D\n\n```bash\n<@input>\n```\n";
        let hover = compute_hover(content, pos(8, 3), &empty_config_vars()).unwrap();
        let HoverContents::Markup(markup) = hover.contents else {
            panic!("expected markup");
        };

        assert!(
            markup.value.contains("hint** (deprecated)"),
            "{}",
            markup.value
        );
    }

    #[test]
    fn frontmatter_completion_ignores_the_request_column() {
        // Frontmatter key completion is line-based, so it must not depend on a
        // convertible column. On `name: \u{1f600}` the emoji spans UTF-16
        // columns 6..8, so column 7 is inside the surrogate pair and has no
        // byte offset at all.
        let content = "---\nname: \u{1f600}\ndesc\n---\n## D\n\n```bash\na\n```\n";
        let response =
            compute_completions(content, pos(1, 7), &empty_config_vars()).expect("completions");
        let CompletionResponse::Array(items) = response else {
            panic!("expected array");
        };

        assert!(items.iter().any(|item| item.label == "name"), "{items:?}");
    }

    #[test]
    fn completion_after_unicode_scalar_does_not_panic() {
        let content = "## D\n\n```bash\ncafé\n```\n";

        assert!(compute_completions(content, pos(3, 4), &empty_config_vars()).is_none());
    }

    #[test]
    fn hover_definition_and_references_use_utf16_ranges() {
        let content = "---\nvariables:\n  bucket:\n    suggestions: [a]\n---\n## D\n\n```bash\né <@bucket> <@key:ls <#bucket>>\n```\n";
        let uri = Url::parse("file:///x.md").unwrap();
        let line_idx = 8;
        let line = content.lines().nth(line_idx).unwrap();
        let hash_start = line.find("<#bucket").unwrap();
        let cursor = position::byte_column_to_utf16(line, hash_start + 2);

        let hover =
            compute_hover(content, pos(line_idx as u32, cursor), &empty_config_vars()).unwrap();
        let hover_range = hover.range.unwrap();
        assert_eq!(
            hover_range.start.character,
            position::byte_column_to_utf16(line, hash_start)
        );
        assert_eq!(
            hover_range.end.character,
            position::byte_column_to_utf16(line, hash_start + "<#bucket>".len())
        );

        let definition = compute_definition(
            &uri,
            content,
            pos(line_idx as u32, cursor),
            &empty_app_config(),
        )
        .unwrap();
        let GotoDefinitionResponse::Scalar(location) = definition else {
            panic!("expected scalar");
        };
        assert_eq!(location.range.start.line, 2);

        let references = compute_references(&uri, content, pos(line_idx as u32, cursor)).unwrap();
        let first_placeholder = references
            .iter()
            .find(|location| {
                location.range.start.line == line_idx as u32 && location.range.start.character == 2
            })
            .expect("reference after unicode prefix");
        assert_eq!(first_placeholder.range.end.character, 10);
    }
}
