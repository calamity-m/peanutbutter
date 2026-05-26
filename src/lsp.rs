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
use crate::domain::VariableSpec;
use crate::lint;
use crate::parser;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use tower_lsp::jsonrpc::Result;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer, LspService, Server};

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

#[tower_lsp::async_trait]
impl LanguageServer for Backend {
    async fn initialize(&self, _params: InitializeParams) -> Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: ServerCapabilities {
                text_document_sync: Some(TextDocumentSyncCapability::Kind(
                    TextDocumentSyncKind::FULL,
                )),
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
                ..Default::default()
            },
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
        Ok(compute_completions(content, pos, &self.config.variables))
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
        Ok(compute_hover(content, pos, &self.config.variables))
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
        Ok(compute_definition(uri, content, pos, self.config.as_ref()))
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
        Ok(compute_references(uri, content, pos))
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

// ---------------------------------------------------------------------------
// Completions
// ---------------------------------------------------------------------------

/// Known top-level frontmatter keys with documentation.
const FRONTMATTER_KEYS: &[(&str, &str)] = &[
    ("name", "Human-readable title for this snippet file"),
    (
        "description",
        "Short prose description of the file contents",
    ),
    ("tags", "Searchable tags (e.g. `[git, shell]`)"),
    ("variables", "File-local variable input specifications"),
];

/// Known sub-keys under `variables.<name>:` with documentation.
const VARIABLE_SPEC_KEYS: &[(&str, &str)] = &[
    (
        "default",
        "Pre-populated value shown in the prompt input box",
    ),
    (
        "suggestions",
        "Fixed suggestion values shown in the suggestion list",
    ),
    (
        "command",
        "Shell command whose stdout lines are used as suggestions",
    ),
];

fn compute_completions(
    content: &str,
    pos: Position,
    config_vars: &BTreeMap<String, VariableSpec>,
) -> Option<CompletionResponse> {
    let lines: Vec<&str> = content.lines().collect();
    let line_idx = pos.line as usize;
    let char_idx = pos.character as usize;
    let current_line = lines.get(line_idx).copied().unwrap_or("");

    // Detect context
    let fm_end = frontmatter_end_line(&lines);
    let in_frontmatter = fm_end
        .map(|end| line_idx > 0 && line_idx < end)
        .unwrap_or(false);

    if in_frontmatter {
        // Are we inside a `variables:` block (indented)?
        let indent = current_line.len() - current_line.trim_start().len();
        if indent >= 2 {
            // Could be inside a variable spec; check if parent block is `variables:`.
            let in_var_block = lines[..line_idx]
                .iter()
                .rev()
                .any(|l| l.trim() == "variables:");
            if in_var_block {
                // Completing variable spec sub-keys or an inner list item
                let trimmed = current_line.trim_start();
                let prefix = trimmed.split(':').next().unwrap_or("").trim();
                let items = VARIABLE_SPEC_KEYS
                    .iter()
                    .filter(|(k, _)| k.starts_with(prefix))
                    .map(|(k, doc)| completion_item(k, doc, CompletionItemKind::FIELD))
                    .collect();
                return Some(CompletionResponse::Array(items));
            }
        }
        // Top-level frontmatter key completion
        let prefix = current_line
            .trim_start()
            .split(':')
            .next()
            .unwrap_or("")
            .trim();
        let items = FRONTMATTER_KEYS
            .iter()
            .filter(|(k, _)| k.starts_with(prefix))
            .map(|(k, doc)| completion_item(k, doc, CompletionItemKind::FIELD))
            .collect();
        return Some(CompletionResponse::Array(items));
    }

    let before_cursor = &current_line[..char_idx.min(current_line.len())];

    // Offer `<#variable>` dependent-ref completions when user typed `<#`.
    // These are valid inside a suggestion command source (which we cannot
    // easily detect from raw text), so we offer them whenever the cursor
    // follows a `<#` token.
    if let Some(at_pos) = before_cursor.rfind("<#") {
        let var_prefix = &before_cursor[at_pos + 2..];
        let parsed =
            parser::parse_file(std::path::Path::new(""), std::path::Path::new(""), content);
        let mut names: Vec<String> = parsed.frontmatter.variables.keys().cloned().collect();
        // Add config-defined names not already in frontmatter.
        for name in config_vars.keys() {
            if !names.contains(name) {
                names.push(name.clone());
            }
        }
        // Add body placeholder names too, in document order.
        for snippet in &parsed.snippets {
            for v in &snippet.variables {
                if !names.contains(&v.name) {
                    names.push(v.name.clone());
                }
            }
        }
        if let Some(owner) = enclosing_placeholder_owner(before_cursor, at_pos)
            && let Some(snippet) = snippet_at_line(content, line_idx)
        {
            let mut earlier = Vec::new();
            for v in &snippet.variables {
                if v.name == owner {
                    break;
                }
                if !earlier.contains(&v.name) {
                    earlier.push(v.name.clone());
                }
            }
            names.retain(|name| earlier.contains(name));
        }
        let items: Vec<CompletionItem> = names
            .into_iter()
            .filter(|name| name.starts_with(var_prefix))
            .map(|name| {
                let detail = parsed
                    .frontmatter
                    .variables
                    .get(&name)
                    .or_else(|| config_vars.get(&name))
                    .map(variable_spec_summary)
                    .unwrap_or_else(|| "dependent reference".to_string());
                let mut item = completion_item(&name, &detail, CompletionItemKind::VARIABLE);
                item.insert_text = Some(format!("{name}>"));
                item.insert_text_format = Some(InsertTextFormat::PLAIN_TEXT);
                item
            })
            .collect();
        return Some(CompletionResponse::Array(items));
    }

    // Inside a code block — offer `<@variable>` completions when user typed `<@`
    if before_cursor.ends_with("<@") || before_cursor.contains("<@") {
        // Extract the prefix after `<@`
        let at_pos = before_cursor.rfind("<@").map(|i| i + 2).unwrap_or(0);
        let var_prefix = &before_cursor[at_pos..];
        let parsed =
            parser::parse_file(std::path::Path::new(""), std::path::Path::new(""), content);
        // Merge frontmatter variables with config-defined variables; frontmatter takes priority.
        let mut all_vars: BTreeMap<&str, &VariableSpec> =
            config_vars.iter().map(|(k, v)| (k.as_str(), v)).collect();
        for (k, v) in &parsed.frontmatter.variables {
            all_vars.insert(k.as_str(), v);
        }
        let items: Vec<CompletionItem> = all_vars
            .into_iter()
            .filter(|(name, _)| name.starts_with(var_prefix))
            .map(|(name, spec)| {
                let detail = variable_spec_summary(spec);
                let mut item = completion_item(name, &detail, CompletionItemKind::VARIABLE);
                item.insert_text = Some(format!("{name}>"));
                item.insert_text_format = Some(InsertTextFormat::PLAIN_TEXT);
                item
            })
            .collect();
        return Some(CompletionResponse::Array(items));
    }

    None
}

fn completion_item(label: &str, documentation: &str, kind: CompletionItemKind) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        documentation: Some(Documentation::String(documentation.to_string())),
        ..Default::default()
    }
}

fn variable_spec_summary(spec: &crate::domain::VariableSpec) -> String {
    let mut parts = Vec::new();
    if let Some(d) = &spec.default {
        parts.push(format!("default: `{d}`"));
    }
    if !spec.suggestions.is_empty() {
        parts.push(format!("suggestions: {}", spec.suggestions.join(", ")));
    }
    if let Some(cmd) = &spec.command {
        parts.push(format!("command: `{cmd}`"));
    }
    if parts.is_empty() {
        "free-form input".to_string()
    } else {
        parts.join("\n")
    }
}

// ---------------------------------------------------------------------------
// Hover
// ---------------------------------------------------------------------------

fn compute_hover(
    content: &str,
    pos: Position,
    config_vars: &BTreeMap<String, VariableSpec>,
) -> Option<Hover> {
    let lines: Vec<&str> = content.lines().collect();
    let line_idx = pos.line as usize;
    let char_idx = pos.character as usize;
    let current_line = lines.get(line_idx).copied()?;

    let fm_end = frontmatter_end_line(&lines);
    let in_frontmatter = fm_end
        .map(|end| line_idx > 0 && line_idx < end)
        .unwrap_or(false);

    if in_frontmatter {
        return hover_frontmatter_key(current_line, pos);
    }

    // Prefer `<#name>` (inner) over `<@name>` (potentially enclosing).
    if let Some(h) = hover_dependent_ref(content, current_line, char_idx, pos, config_vars) {
        return Some(h);
    }
    hover_variable_placeholder(content, current_line, char_idx, pos, config_vars)
}

fn hover_dependent_ref(
    content: &str,
    line: &str,
    char_idx: usize,
    pos: Position,
    config_vars: &BTreeMap<String, VariableSpec>,
) -> Option<Hover> {
    let (name, start, end, raw) = dependent_ref_at(line, char_idx)?;
    let parsed = parser::parse_file(std::path::Path::new(""), std::path::Path::new(""), content);
    let mut md = if raw {
        format!("**`<#{name}:raw>`** — dependent reference (raw splice, **not quoted**)\n\n")
    } else {
        format!("**`<#{name}>`** — dependent reference (shell-quoted)\n\n")
    };
    let spec = parsed
        .frontmatter
        .variables
        .get(&name)
        .or_else(|| config_vars.get(&name));
    if let Some(spec) = spec {
        if let Some(d) = &spec.default {
            md.push_str(&format!("- **default**: `{d}`\n"));
        }
        if !spec.suggestions.is_empty() {
            md.push_str(&format!(
                "- **suggestions**: {}\n",
                spec.suggestions.join(", ")
            ));
        }
        if let Some(cmd) = &spec.command {
            md.push_str(&format!("- **command**: `{cmd}`\n"));
        }
    } else {
        md.push_str("_no variable spec; declared inline_\n");
    }
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: md,
        }),
        range: Some(line_range(pos.line, start as u32, end as u32)),
    })
}

fn hover_frontmatter_key(line: &str, pos: Position) -> Option<Hover> {
    let key = line.trim_start().split(':').next()?.trim();
    let doc = FRONTMATTER_KEYS
        .iter()
        .find(|(k, _)| *k == key)
        .map(|(_, doc)| *doc)?;
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("**`{key}`** — {doc}"),
        }),
        range: Some(line_range(pos.line, 0, key.len() as u32)),
    })
}

fn hover_variable_placeholder(
    content: &str,
    line: &str,
    char_idx: usize,
    pos: Position,
    config_vars: &BTreeMap<String, VariableSpec>,
) -> Option<Hover> {
    let (name, start, end) = placeholder_at(line, char_idx)?;
    let parsed = parser::parse_file(std::path::Path::new(""), std::path::Path::new(""), content);
    let spec = parsed
        .frontmatter
        .variables
        .get(&name)
        .or_else(|| config_vars.get(&name))?;
    let mut md = format!("**`<@{name}>`**\n\n");
    if let Some(d) = &spec.default {
        md.push_str(&format!("- **default**: `{d}`\n"));
    }
    if !spec.suggestions.is_empty() {
        md.push_str(&format!(
            "- **suggestions**: {}\n",
            spec.suggestions.join(", ")
        ));
    }
    if let Some(cmd) = &spec.command {
        md.push_str(&format!("- **command**: `{cmd}`\n"));
    }
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: md,
        }),
        range: Some(line_range(pos.line, start as u32, end as u32)),
    })
}

// ---------------------------------------------------------------------------
// Go-to-definition
// ---------------------------------------------------------------------------

fn compute_definition(
    uri: &Url,
    content: &str,
    pos: Position,
    config: &AppConfig,
) -> Option<GotoDefinitionResponse> {
    let lines: Vec<&str> = content.lines().collect();
    let line_idx = pos.line as usize;
    let char_idx = pos.character as usize;
    let current_line = lines.get(line_idx).copied()?;

    // Cursor may be on either `<@name>` or `<#name>`. Prefer the inner
    // `<#name>` token because `placeholder_at` will match an enclosing
    // `<@key:cmd-with-<#name>>` even when the cursor is on the nested ref.
    let name = dependent_ref_at(current_line, char_idx)
        .map(|(n, ..)| n)
        .or_else(|| placeholder_at(current_line, char_idx).map(|(n, ..)| n))?;

    // Prefer frontmatter declaration in the current file.
    if let Some(def_line) = find_variable_declaration_line(&lines, &name) {
        let target_range = line_range(def_line as u32, 0, lines[def_line].len() as u32);
        return Some(GotoDefinitionResponse::Scalar(Location {
            uri: uri.clone(),
            range: target_range,
        }));
    }

    // Fall back to the config file if the variable is declared there.
    if config.variables.contains_key(&name)
        && let Some(loc) = find_config_variable_location(&config.paths.config_file, &name)
    {
        return Some(GotoDefinitionResponse::Scalar(loc));
    }

    None
}

/// Find the location of `[variables.<name>]` in the config TOML file.
fn find_config_variable_location(config_file: &Path, name: &str) -> Option<Location> {
    let content = std::fs::read_to_string(config_file).ok()?;
    let target = format!("[variables.{name}]");
    let (line_idx, _) = content
        .lines()
        .enumerate()
        .find(|(_, line)| line.trim() == target)?;
    let uri = Url::from_file_path(config_file).ok()?;
    let range = line_range(line_idx as u32, 0, target.len() as u32);
    Some(Location { uri, range })
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
// Find references
// ---------------------------------------------------------------------------

fn compute_references(uri: &Url, content: &str, pos: Position) -> Option<Vec<Location>> {
    let lines: Vec<&str> = content.lines().collect();
    let line_idx = pos.line as usize;
    let current_line = lines.get(line_idx).copied()?;

    // If cursor is on the variable declaration in frontmatter, find all `<@name>` usages.
    let fm_end = frontmatter_end_line(&lines)?;
    let in_frontmatter = line_idx > 0 && line_idx < fm_end;

    let var_name: String;
    if in_frontmatter {
        // Check if this line is a variable declaration line (inside `variables:` block).
        let trimmed = current_line.trim_start();
        var_name = trimmed.split(':').next()?.trim().to_string();
        // Verify it is actually declared in frontmatter variables.
        let parsed =
            parser::parse_file(std::path::Path::new(""), std::path::Path::new(""), content);
        if !parsed.frontmatter.variables.contains_key(&var_name) {
            return None;
        }
    } else {
        // Cursor might be on a `<#name>` ref or a `<@name>` placeholder.
        // Prefer the inner `<#name>` so nested refs inside `<@key:...>` work.
        let char_idx = pos.character as usize;
        var_name = dependent_ref_at(current_line, char_idx)
            .map(|(n, ..)| n)
            .or_else(|| placeholder_at(current_line, char_idx).map(|(n, ..)| n))?;
    }

    let mut locations = Vec::new();
    // Find all `<@var_name>` and `<@var_name:...>` placeholder occurrences.
    let at_pattern = format!("<@{var_name}");
    // Find all `<#var_name>` and `<#var_name:raw>` dependent-ref occurrences.
    let hash_pattern = format!("<#{var_name}");
    for (i, line) in lines.iter().enumerate() {
        for pattern in [&at_pattern, &hash_pattern] {
            let mut search_from = 0;
            while let Some(col) = line[search_from..].find(pattern.as_str()) {
                let abs_col = search_from + col;
                let end_col = abs_col + pattern.len();
                // Boundary check: the next char must be `>` or `:` to avoid
                // matching `<@foo` inside `<@foobar>`.
                let next = line[end_col..].chars().next();
                if matches!(next, Some('>') | Some(':')) {
                    locations.push(Location {
                        uri: uri.clone(),
                        range: line_range(i as u32, abs_col as u32, end_col as u32),
                    });
                }
                search_from = abs_col + 1;
            }
        }
    }
    Some(locations)
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn enclosing_placeholder_owner(before_cursor: &str, hash_pos: usize) -> Option<String> {
    let at_pos = before_cursor[..hash_pos].rfind("<@")?;
    let inner = &before_cursor[at_pos + 2..];
    let name = inner.split(':').next()?.trim();
    if name.is_empty() {
        None
    } else {
        Some(name.to_string())
    }
}

fn snippet_at_line(content: &str, line_idx: usize) -> Option<crate::domain::Snippet> {
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
fn frontmatter_end_line(lines: &[&str]) -> Option<usize> {
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
fn dependent_ref_at(line: &str, char_idx: usize) -> Option<(String, usize, usize, bool)> {
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
fn placeholder_at(line: &str, char_idx: usize) -> Option<(String, usize, usize)> {
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
fn line_range(line: u32, start_char: u32, end_char: u32) -> Range {
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
