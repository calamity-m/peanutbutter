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
use std::collections::HashMap;
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
        if find_marker_root(&uri_to_path(uri)).is_none() {
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
        if find_marker_root(&uri_to_path(uri)).is_none() {
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
        if find_marker_root(&uri_to_path(uri)).is_none() {
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
        if find_marker_root(&uri_to_path(uri)).is_none() {
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
        if find_marker_root(&uri_to_path(uri)).is_none() {
            return Ok(None);
        }
        let docs = self.documents.read().await;
        let Some(content) = docs.get(uri) else {
            return Ok(None);
        };
        Ok(code_actions::compute_code_actions(
            uri,
            content,
            params.range,
            &self.config.variables,
        ))
    }

    async fn semantic_tokens_full(
        &self,
        params: SemanticTokensParams,
    ) -> Result<Option<SemanticTokensResult>> {
        let uri = &params.text_document.uri;
        if find_marker_root(&uri_to_path(uri)).is_none() {
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
        let Some(root) = find_marker_root(&path) else {
            // Outside a peanutbutter snippet tree; clear any stale diagnostics.
            self.client
                .publish_diagnostics(uri.clone(), vec![], None)
                .await;
            return;
        };
        let findings = lint::lint_file(&path, &root, content, &self.config);
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
    let line_len = content
        .lines()
        .nth(line as usize)
        .map(|l| l.len() as u32)
        .unwrap_or(0);
    let range = match (finding.col_start, finding.col_end) {
        (Some(c0), Some(c1)) => Range {
            start: Position {
                line,
                character: c0 as u32,
            },
            end: Position {
                line,
                character: c1 as u32,
            },
        },
        _ => Range {
            start: Position { line, character: 0 },
            end: Position {
                line,
                character: line_len,
            },
        },
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

/// Walk up from `path`'s parent directory looking for any [`MARKER_FILENAMES`].
///
/// Returns the directory that contains the marker file, which becomes the
/// snippet root used for linting and snippet ID construction. Returns `None`
/// when no marker is found before reaching the filesystem root.
fn find_marker_root(path: &Path) -> Option<PathBuf> {
    let mut dir = path.parent()?;
    loop {
        for name in MARKER_FILENAMES {
            if dir.join(name).exists() {
                return Some(dir.to_path_buf());
            }
        }
        match dir.parent() {
            Some(parent) => dir = parent,
            None => return None,
        }
    }
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
            let end = match bytes[start..].iter().position(|&b| b == b'>') {
                Some(rel) => start + rel + 1,
                None => bytes.len(),
            };
            if char_idx >= start && char_idx <= end {
                let inner = &line[start + 2..end.min(bytes.len()).saturating_sub(1)];
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
            // Find closing `>`
            let end = match bytes[start..].iter().position(|&b| b == b'>') {
                Some(rel) => start + rel + 1,
                None => bytes.len(),
            };
            if char_idx >= start && char_idx <= end {
                // Extract name: everything between `<@` and the first `:` or `>`
                let inner = &line[start + 2..end.min(bytes.len()) - 1];
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

/// Build a single-line [`Range`] from 0-based line + character column bounds.
pub(super) fn line_range(line: u32, start_char: u32, end_char: u32) -> Range {
    Range {
        start: Position {
            line,
            character: start_char,
        },
        end: Position {
            line,
            character: end_char,
        },
    }
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

    #[test]
    fn find_marker_root_dot_peanutbutter_toml() {
        let root = tmp_dir();
        fs::write(root.join(".peanutbutter.toml"), "").unwrap();
        let file = root.join("sub").join("snippets.md");
        fs::create_dir_all(file.parent().unwrap()).unwrap();
        fs::write(&file, "").unwrap();
        assert_eq!(find_marker_root(&file), Some(root.clone()));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_marker_root_peanutbutter_toml() {
        let root = tmp_dir();
        fs::write(root.join("peanutbutter.toml"), "").unwrap();
        let file = root.join("snippets.md");
        fs::write(&file, "").unwrap();
        assert_eq!(find_marker_root(&file), Some(root.clone()));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_marker_root_underscore_peanutbutter_toml() {
        let root = tmp_dir();
        fs::write(root.join("_peanutbutter.toml"), "").unwrap();
        let file = root.join("snippets.md");
        fs::write(&file, "").unwrap();
        assert_eq!(find_marker_root(&file), Some(root.clone()));
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_marker_root_returns_none_when_no_marker() {
        let root = tmp_dir();
        let file = root.join("snippets.md");
        fs::write(&file, "").unwrap();
        assert_eq!(find_marker_root(&file), None);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn find_marker_root_nearest_ancestor_wins() {
        // Inner marker should win over outer one.
        let outer = tmp_dir();
        let inner = outer.join("sub");
        fs::create_dir_all(&inner).unwrap();
        fs::write(outer.join(".peanutbutter.toml"), "").unwrap();
        fs::write(inner.join("peanutbutter.toml"), "").unwrap();
        let file = inner.join("snippets.md");
        fs::write(&file, "").unwrap();
        assert_eq!(find_marker_root(&file), Some(inner.clone()));
        fs::remove_dir_all(&outer).unwrap();
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
}
