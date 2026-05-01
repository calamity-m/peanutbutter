use super::app::Screen;
use super::*;
use crate::domain::{Frontmatter, Snippet, SnippetFile, Variable, VariableSource};
use crate::frecency::FrecencyStore;
use crate::index::SnippetIndex;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::Line;
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

#[derive(Default)]
struct TestProvider {
    values: RefCell<HashMap<String, Vec<String>>>,
}

impl TestProvider {
    fn with(self, name: &str, values: &[&str]) -> Self {
        self.values.borrow_mut().insert(
            name.to_string(),
            values.iter().map(|value| value.to_string()).collect(),
        );
        self
    }
}

impl SuggestionProvider for TestProvider {
    fn suggestions(&self, variable: &Variable, _cwd: &Path) -> io::Result<Vec<String>> {
        Ok(self
            .values
            .borrow()
            .get(&variable.name)
            .cloned()
            .unwrap_or_default())
    }

    fn default_input(&self, _variable: &Variable) -> Option<String> {
        None
    }
}

fn snippet_file(rel: &str, name: &str, body: &str, variables: Vec<Variable>) -> SnippetFile {
    SnippetFile {
        path: PathBuf::from(rel),
        relative_path: PathBuf::from(rel),
        frontmatter: Frontmatter::default(),
        snippets: vec![Snippet {
            id: crate::domain::SnippetId::new(rel, "slug"),
            name: name.to_string(),
            description: "desc".to_string(),
            body: body.to_string(),
            variables,
        }],
    }
}

fn snippet_file_with_slug(rel: &str, slug: &str, name: &str, body: &str) -> SnippetFile {
    SnippetFile {
        path: PathBuf::from(rel),
        relative_path: PathBuf::from(rel),
        frontmatter: Frontmatter::default(),
        snippets: vec![Snippet {
            id: crate::domain::SnippetId::new(rel, slug),
            name: name.to_string(),
            description: "desc".to_string(),
            body: body.to_string(),
            variables: vec![],
        }],
    }
}

fn app_with_body(
    body: &str,
    variables: Vec<Variable>,
    provider: TestProvider,
) -> ExecutionApp<TestProvider> {
    let index = SnippetIndex::from_files([snippet_file("x.md", "Demo", body, variables)]);
    let frecency = FrecencyStore::new();
    ExecutionApp::new(
        index,
        frecency,
        PathBuf::from("."),
        0,
        crate::config::SearchConfig::default(),
        crate::config::Theme::default(),
        provider,
    )
}

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn completed(event: AppEvent) -> ExecutionOutcome {
    match event {
        AppEvent::Completed(outcome) => outcome,
        AppEvent::Continue => panic!("expected completed event, got continue"),
        AppEvent::EditSnippet(id) => panic!("expected completed event, got edit request for {id}"),
        AppEvent::Cancelled => panic!("expected completed event, got cancelled"),
    }
}

fn edit_requested(event: AppEvent) -> crate::domain::SnippetId {
    match event {
        AppEvent::EditSnippet(id) => id,
        AppEvent::Continue => panic!("expected edit request, got continue"),
        AppEvent::Cancelled => panic!("expected edit request, got cancelled"),
        AppEvent::Completed(_) => panic!("expected edit request, got completed"),
    }
}

fn line_text(line: &Line<'_>) -> String {
    line.spans
        .iter()
        .map(|span| span.content.as_ref())
        .collect()
}

#[test]
fn render_command_replaces_each_placeholder_form() {
    let mut values = BTreeMap::new();
    values.insert("file".to_string(), "Cargo.toml".to_string());
    values.insert("pattern".to_string(), "needle".to_string());
    values.insert("method".to_string(), "POST".to_string());
    let rendered = render_command(
        "cat <@file> | grep <@pattern:?hi> && curl -X <@method:echo GET>",
        &values,
    );
    assert_eq!(rendered, "cat Cargo.toml | grep needle && curl -X POST");
}

#[test]
fn render_command_keeps_unresolved_placeholders() {
    let values = BTreeMap::new();
    let rendered = render_command("echo <@missing>", &values);
    assert_eq!(rendered, "echo <@missing>");
}

fn span_style_for(line: &Line<'_>, content: &str) -> ratatui::style::Style {
    line.spans
        .iter()
        .find(|span| span.content.as_ref() == content)
        .unwrap_or_else(|| panic!("expected a span with content {content:?}"))
        .style
}

#[test]
fn render_command_text_highlights_active_value() {
    let mut values = BTreeMap::new();
    values.insert("file".to_string(), "Cargo.toml".to_string());
    let theme = crate::config::Theme::default();
    let rendered = render_command_text("cat <@file>", &values, Some("file"), &theme);
    assert_eq!(line_text(&rendered.lines[0]), "cat Cargo.toml");
    assert_eq!(
        span_style_for(&rendered.lines[0], "Cargo.toml"),
        active_prompt_style(&theme)
    );
}

#[test]
fn render_command_text_highlights_active_placeholder_and_dims_others() {
    let values = BTreeMap::new();
    let theme = crate::config::Theme::default();
    let rendered =
        render_command_text("echo <@missing> <@later>", &values, Some("missing"), &theme);
    assert_eq!(line_text(&rendered.lines[0]), "echo <@missing> <@later>");
    assert_eq!(
        span_style_for(&rendered.lines[0], "<@missing>"),
        active_prompt_style(&theme)
    );
    assert_eq!(
        span_style_for(&rendered.lines[0], "<@later>"),
        placeholder_prompt_style(&theme)
    );
}

#[test]
fn compact_viewport_height_uses_configured_height() {
    assert_eq!(compact_viewport_height(12), 12);
    assert_eq!(compact_viewport_height(4), 4);
    assert_eq!(compact_viewport_height(20), 20);
}

#[test]
fn compact_viewport_height_enforces_minimum_of_one() {
    assert_eq!(compact_viewport_height(0), 1);
}

#[test]
fn unique_variables_prompt_only_once_for_duplicate_names() {
    let variables = vec![
        Variable {
            name: "file".to_string(),
            source: VariableSource::Free,
        },
        Variable {
            name: "file".to_string(),
            source: VariableSource::Free,
        },
    ];
    let uniq = unique_variables(&variables);
    assert_eq!(uniq.len(), 1);
    assert_eq!(uniq[0].name, "file");
}

#[test]
fn built_in_file_and_directory_sources_list_cwd_entries() {
    let dir = std::env::temp_dir().join(format!("pb-execute-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("subdir")).unwrap();
    fs::write(dir.join("alpha.txt"), "hi").unwrap();
    fs::write(dir.join("beta.txt"), "hi").unwrap();

    let files = builtin_suggestions("file", &dir).unwrap();
    let dirs = builtin_suggestions("directory", &dir).unwrap();
    assert_eq!(files, vec!["alpha.txt".to_string(), "beta.txt".to_string()]);
    assert_eq!(dirs, vec!["subdir".to_string()]);

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn command_suggestions_split_literal_backslash_n_sequences() {
    let dir = Path::new(".");
    let values = command_suggestions("printf 'GET\\\\nPOST\\\\nPUT'", dir).unwrap();
    assert_eq!(
        values,
        vec!["GET".to_string(), "POST".to_string(), "PUT".to_string()]
    );
}

#[test]
fn enter_from_picker_completes_snippet_with_no_variables() {
    let mut app = app_with_body("echo hi", vec![], TestProvider::default());
    let outcome = completed(app.handle_key(press(KeyCode::Enter)));
    assert_eq!(outcome.command, "echo hi");
}

#[test]
fn ctrl_e_from_fuzzy_requests_edit_for_selected_snippet() {
    let mut app = app_with_body("echo hi", vec![], TestProvider::default());
    let id =
        edit_requested(app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL)));
    assert_eq!(id.as_str(), "x.md#slug");
}

#[test]
fn ctrl_e_from_browse_requests_edit_for_selected_snippet() {
    let mut app = app_with_body("echo hi", vec![], TestProvider::default());
    app.nav_mode = NavigationMode::Browse;
    app.browse.path = vec!["x.md".to_string()];
    app.browse.selection = Some(0);

    let id =
        edit_requested(app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL)));
    assert_eq!(id.as_str(), "x.md#slug");
}

#[test]
fn ctrl_e_from_browse_directory_does_not_request_edit() {
    let mut app = app_with_body("echo hi", vec![], TestProvider::default());
    app.nav_mode = NavigationMode::Browse;
    app.browse.selection = Some(0);

    let event = app.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::CONTROL));
    assert!(matches!(event, AppEvent::Continue));
    assert_eq!(app.browse.path, Vec::<String>::new());
}

#[test]
fn replace_index_preserves_fuzzy_query_and_selects_previous_snippet() {
    let mut app = ExecutionApp::new(
        SnippetIndex::from_files([
            snippet_file_with_slug("git.md", "status", "Git Status", "git status"),
            snippet_file_with_slug("docker.md", "ps", "Docker Ps", "docker ps"),
        ]),
        FrecencyStore::new(),
        PathBuf::from("."),
        0,
        crate::config::SearchConfig::default(),
        crate::config::Theme::default(),
        TestProvider::default(),
    );
    app.fuzzy.set_query("docker");
    let previous_id = crate::domain::SnippetId::new("docker.md", "ps");

    let found = app.replace_index(
        SnippetIndex::from_files([
            snippet_file_with_slug("git.md", "status", "Git Status", "git status"),
            snippet_file_with_slug("docker.md", "ps", "Docker Ps", "docker ps -a"),
        ]),
        Some(&previous_id),
    );

    assert!(found);
    assert_eq!(app.fuzzy.query, "docker");
    assert_eq!(
        app.selected_snippet().map(|snippet| snippet.id().as_str()),
        Some("docker.md#ps")
    );
}

#[test]
fn replace_index_reports_when_previous_snippet_is_missing() {
    let mut app = app_with_body("echo hi", vec![], TestProvider::default());
    let previous_id = crate::domain::SnippetId::new("missing.md", "gone");

    let found = app.replace_index(
        SnippetIndex::from_files([snippet_file_with_slug("x.md", "slug", "Demo", "echo hi")]),
        Some(&previous_id),
    );

    assert!(!found);
    assert_eq!(
        app.selected_snippet().map(|snippet| snippet.id().as_str()),
        Some("x.md#slug")
    );
}

#[test]
fn replace_index_preserves_query_when_previous_snippet_no_longer_matches() {
    let mut app = ExecutionApp::new(
        SnippetIndex::from_files([snippet_file_with_slug(
            "ops.md",
            "ps",
            "Docker Ps",
            "docker ps",
        )]),
        FrecencyStore::new(),
        PathBuf::from("."),
        0,
        crate::config::SearchConfig::default(),
        crate::config::Theme::default(),
        TestProvider::default(),
    );
    app.fuzzy.set_query("docker");
    let previous_id = crate::domain::SnippetId::new("ops.md", "ps");

    let found = app.replace_index(
        SnippetIndex::from_files([snippet_file_with_slug(
            "ops.md",
            "ps",
            "Pods",
            "kubectl get pods",
        )]),
        Some(&previous_id),
    );

    assert!(found);
    assert_eq!(app.fuzzy.query, "docker");
    assert!(app.selected_snippet().is_none());
}

#[test]
fn replace_index_selects_previous_browse_snippet_when_still_visible() {
    let mut app = ExecutionApp::new(
        SnippetIndex::from_files([snippet_file_with_slug(
            "git/commands.md",
            "status",
            "Status",
            "git status",
        )]),
        FrecencyStore::new(),
        PathBuf::from("."),
        0,
        crate::config::SearchConfig::default(),
        crate::config::Theme::default(),
        TestProvider::default(),
    );
    app.nav_mode = NavigationMode::Browse;
    app.browse.path = vec!["git".to_string(), "commands.md".to_string()];
    app.browse.selection = Some(0);
    let previous_id = crate::domain::SnippetId::new("git/commands.md", "status");

    let found = app.replace_index(
        SnippetIndex::from_files([snippet_file_with_slug(
            "git/commands.md",
            "status",
            "Status",
            "git status --short",
        )]),
        Some(&previous_id),
    );

    assert!(found);
    assert_eq!(
        app.selected_snippet().map(|snippet| snippet.body()),
        Some("git status --short")
    );
}

#[test]
fn replace_index_climbs_missing_browse_directory() {
    let mut app = ExecutionApp::new(
        SnippetIndex::from_files([snippet_file_with_slug(
            "old/place.md",
            "demo",
            "Demo",
            "echo old",
        )]),
        FrecencyStore::new(),
        PathBuf::from("."),
        0,
        crate::config::SearchConfig::default(),
        crate::config::Theme::default(),
        TestProvider::default(),
    );
    app.nav_mode = NavigationMode::Browse;
    app.browse.path = vec!["old".to_string(), "place.md".to_string()];
    app.browse.selection = Some(0);
    let previous_id = crate::domain::SnippetId::new("old/place.md", "demo");

    let found = app.replace_index(
        SnippetIndex::from_files([snippet_file_with_slug(
            "new/place.md",
            "demo",
            "Demo",
            "echo new",
        )]),
        Some(&previous_id),
    );

    assert!(!found);
    assert_eq!(app.browse.path, Vec::<String>::new());
    assert_eq!(app.browse.selection, Some(0));
}

#[test]
fn prompt_esc_returns_to_select_preserving_query() {
    let variables = vec![Variable {
        name: "x".to_string(),
        source: VariableSource::Free,
    }];
    let mut app = app_with_body("echo <@x>", variables, TestProvider::default());
    app.fuzzy.set_query("Demo");
    let _ = app.handle_key(press(KeyCode::Enter));
    assert!(matches!(app.screen, Screen::Prompt(_)));
    let _ = app.handle_key(press(KeyCode::Esc));
    assert!(matches!(app.screen, Screen::Select));
    assert_eq!(app.fuzzy.query, "Demo");
}

#[test]
fn prompt_esc_in_browse_mode_preserves_browse_position() {
    let variables = vec![Variable {
        name: "x".to_string(),
        source: VariableSource::Free,
    }];
    let file = SnippetFile {
        path: PathBuf::from("git/commits.md"),
        relative_path: PathBuf::from("git/commits.md"),
        frontmatter: crate::domain::Frontmatter::default(),
        snippets: vec![crate::domain::Snippet {
            id: crate::domain::SnippetId::new("git/commits.md", "slug"),
            name: "Log".to_string(),
            description: "desc".to_string(),
            body: "git log <@x>".to_string(),
            variables,
        }],
    };
    let index = SnippetIndex::from_files([file]);
    let frecency = FrecencyStore::new();
    let mut app = ExecutionApp::new(
        index,
        frecency,
        PathBuf::from("."),
        0,
        crate::config::SearchConfig::default(),
        crate::config::Theme::default(),
        TestProvider::default(),
    );
    app.nav_mode = NavigationMode::Browse;
    app.browse.path = vec!["git".to_string(), "commits.md".to_string()];
    app.browse.input = String::new();
    app.browse.selection = Some(0);

    let _ = app.handle_key(press(KeyCode::Enter));
    assert!(matches!(app.screen, Screen::Prompt(_)));

    let _ = app.handle_key(press(KeyCode::Esc));
    assert!(matches!(app.screen, Screen::Select));
    assert_eq!(
        app.browse.path,
        vec!["git".to_string(), "commits.md".to_string()]
    );
    assert_eq!(app.browse.input, "");
    assert_eq!(app.browse.selection, Some(0));
}

#[test]
fn ctrl_t_toggles_between_search_and_browse() {
    let mut app = app_with_body("echo hi", vec![], TestProvider::default());
    let _ = app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
    assert_eq!(app.navigation_mode(), NavigationMode::Browse);
    let _ = app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
    assert_eq!(app.navigation_mode(), NavigationMode::Fuzzy);
}

#[test]
fn variable_flow_accepts_default_and_emits_rendered_command() {
    let variables = vec![Variable {
        name: "target".to_string(),
        source: VariableSource::Default("world".to_string()),
    }];
    let mut app = app_with_body(
        "echo hello <@target:?world>",
        variables,
        TestProvider::default(),
    );
    let _ = app.handle_key(press(KeyCode::Enter));
    let outcome = completed(app.handle_key(press(KeyCode::Enter)));
    assert_eq!(outcome.command, "echo hello world");
}

#[test]
fn variable_flow_accepts_default_suggestion() {
    let variables = vec![Variable {
        name: "method".to_string(),
        source: VariableSource::Command("ignored".to_string()),
    }];
    let provider = TestProvider::default().with("method", &["GET", "POST"]);
    let mut app = app_with_body("curl -X <@method:ignored>", variables, provider);
    let _ = app.handle_key(press(KeyCode::Enter));
    let outcome = completed(app.handle_key(press(KeyCode::Enter)));
    assert_eq!(outcome.command, "curl -X GET");
}

#[test]
fn variable_flow_uses_config_defined_inputs() {
    let variables = vec![Variable {
        name: "http_method".to_string(),
        source: VariableSource::Free,
    }];
    let mut configured = BTreeMap::new();
    configured.insert(
        "http_method".to_string(),
        crate::config::VariableInputConfig {
            default: Some("POST".to_string()),
            suggestions: vec!["POST".to_string(), "PUT".to_string()],
            command: None,
        },
    );
    let mut app = ExecutionApp::new(
        SnippetIndex::from_files([snippet_file(
            "x.md",
            "Demo",
            "curl -X <@http_method>",
            variables,
        )]),
        FrecencyStore::new(),
        PathBuf::from("."),
        0,
        crate::config::SearchConfig::default(),
        crate::config::Theme::default(),
        SystemSuggestionProvider::new(configured),
    );

    let _ = app.handle_key(press(KeyCode::Enter));
    let Screen::Prompt(prompt) = &app.screen else {
        panic!("expected prompt");
    };
    assert_eq!(prompt.input, "POST");
    assert_eq!(prompt.suggestions, vec!["POST", "PUT"]);
}

#[test]
fn inline_default_overrides_config_default() {
    let variables = vec![Variable {
        name: "namespace".to_string(),
        source: VariableSource::Default("inline-default".to_string()),
    }];
    let mut configured = BTreeMap::new();
    configured.insert(
        "namespace".to_string(),
        crate::config::VariableInputConfig {
            default: Some("config-default".to_string()),
            suggestions: vec![],
            command: None,
        },
    );
    let mut app = ExecutionApp::new(
        SnippetIndex::from_files([snippet_file(
            "x.md",
            "Demo",
            "kubectl get pods -n <@namespace:?inline-default>",
            variables,
        )]),
        FrecencyStore::new(),
        PathBuf::from("."),
        0,
        crate::config::SearchConfig::default(),
        crate::config::Theme::default(),
        SystemSuggestionProvider::new(configured),
    );

    let _ = app.handle_key(press(KeyCode::Enter));
    let Screen::Prompt(prompt) = &app.screen else {
        panic!("expected prompt");
    };
    assert_eq!(prompt.input, "inline-default");
}

#[test]
fn prompt_tab_cycles_forward_between_variables() {
    let variables = vec![
        Variable {
            name: "one".to_string(),
            source: VariableSource::Free,
        },
        Variable {
            name: "two".to_string(),
            source: VariableSource::Free,
        },
    ];
    let mut app = app_with_body("echo <@one> <@two>", variables, TestProvider::default());
    let _ = app.handle_key(press(KeyCode::Enter));
    let _ = app.handle_key(press(KeyCode::Char('a')));
    let _ = app.handle_key(press(KeyCode::Tab));

    let Screen::Prompt(prompt) = &app.screen else {
        panic!("expected prompt");
    };
    assert_eq!(prompt.current_variable().name, "two");
    assert_eq!(prompt.values.get("one").map(String::as_str), Some("a"));

    let _ = app.handle_key(press(KeyCode::Tab));
    let Screen::Prompt(prompt) = &app.screen else {
        panic!("expected prompt");
    };
    assert_eq!(prompt.current_variable().name, "one");
    assert_eq!(prompt.input, "a");
}

#[test]
fn prompt_tab_fills_highlighted_suggestion_before_cycling() {
    let variables = vec![
        Variable {
            name: "method".to_string(),
            source: VariableSource::Command("ignored".to_string()),
        },
        Variable {
            name: "path".to_string(),
            source: VariableSource::Free,
        },
    ];
    let provider = TestProvider::default().with("method", &["GET", "POST"]);
    let mut app = app_with_body("curl -X <@method> <@path>", variables, provider);
    let _ = app.handle_key(press(KeyCode::Enter));

    // First Tab: fills the input from the highlighted suggestion without cycling.
    let _ = app.handle_key(press(KeyCode::Tab));
    let Screen::Prompt(prompt) = &app.screen else {
        panic!("expected prompt");
    };
    assert_eq!(prompt.current_variable().name, "method");
    assert_eq!(prompt.input, "GET");

    // Second Tab: input already matches the selection, so it cycles forward.
    let _ = app.handle_key(press(KeyCode::Tab));
    let Screen::Prompt(prompt) = &app.screen else {
        panic!("expected prompt");
    };
    assert_eq!(prompt.current_variable().name, "path");
    assert_eq!(prompt.values.get("method").map(String::as_str), Some("GET"));
}

#[test]
fn prompt_shift_tab_cycles_backward_between_variables() {
    let variables = vec![
        Variable {
            name: "one".to_string(),
            source: VariableSource::Free,
        },
        Variable {
            name: "two".to_string(),
            source: VariableSource::Free,
        },
    ];
    let mut app = app_with_body("echo <@one> <@two>", variables, TestProvider::default());
    let _ = app.handle_key(press(KeyCode::Enter));
    let _ = app.handle_key(press(KeyCode::BackTab));

    let Screen::Prompt(prompt) = &app.screen else {
        panic!("expected prompt");
    };
    assert_eq!(prompt.current_variable().name, "two");
}

#[test]
fn prompt_backspace_walks_to_previous_variable() {
    let variables = vec![
        Variable {
            name: "one".to_string(),
            source: VariableSource::Free,
        },
        Variable {
            name: "two".to_string(),
            source: VariableSource::Free,
        },
    ];
    let mut app = app_with_body("echo <@one> <@two>", variables, TestProvider::default());
    let _ = app.handle_key(press(KeyCode::Enter));
    let _ = app.handle_key(press(KeyCode::Char('a')));
    let _ = app.handle_key(press(KeyCode::Enter));
    let _ = app.handle_key(press(KeyCode::Backspace));
    let Screen::Prompt(prompt) = &app.screen else {
        panic!("expected prompt");
    };
    assert_eq!(prompt.current_variable().name, "one");
    assert_eq!(prompt.input, "a");
}

#[test]
fn ctrl_j_scrolls_preview_down() {
    let mut app = app_with_body("echo hi", vec![], TestProvider::default());
    let _ = app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    assert_eq!(app.preview_scroll, 3);
}

#[test]
fn ctrl_down_scrolls_preview_down() {
    let mut app = app_with_body("echo hi", vec![], TestProvider::default());
    let _ = app.handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL));
    assert_eq!(app.preview_scroll, 3);
}

#[test]
fn ctrl_k_scrolls_preview_up() {
    let mut app = app_with_body("echo hi", vec![], TestProvider::default());
    app.preview_scroll = 6;
    let _ = app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    assert_eq!(app.preview_scroll, 3);
}

#[test]
fn ctrl_k_does_not_underflow() {
    let mut app = app_with_body("echo hi", vec![], TestProvider::default());
    let _ = app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    assert_eq!(app.preview_scroll, 0);
}

#[test]
fn navigation_resets_preview_scroll() {
    let mut app = app_with_body("echo hi", vec![], TestProvider::default());
    app.preview_scroll = 9;
    let _ = app.handle_key(press(KeyCode::Up));
    assert_eq!(app.preview_scroll, 0);
}

#[test]
fn scroll_bindings_work_in_browse_mode() {
    let mut app = app_with_body("echo hi", vec![], TestProvider::default());
    app.nav_mode = NavigationMode::Browse;
    let _ = app.handle_key(KeyEvent::new(KeyCode::Char('j'), KeyModifiers::CONTROL));
    assert_eq!(app.preview_scroll, 3);
    let _ = app.handle_key(KeyEvent::new(KeyCode::Char('k'), KeyModifiers::CONTROL));
    assert_eq!(app.preview_scroll, 0);
}

#[test]
fn fuzzy_typing_resets_preview_scroll() {
    let mut app = app_with_body("echo hi", vec![], TestProvider::default());
    app.preview_scroll = 9;
    let _ = app.handle_key(press(KeyCode::Char('h')));
    assert_eq!(app.preview_scroll, 0);
}

#[test]
fn fuzzy_backspace_resets_preview_scroll() {
    let mut app = app_with_body("echo hi", vec![], TestProvider::default());
    app.fuzzy.set_query("h");
    app.preview_scroll = 9;
    let _ = app.handle_key(press(KeyCode::Backspace));
    assert_eq!(app.preview_scroll, 0);
}

#[test]
fn browse_typing_resets_preview_scroll() {
    let mut app = app_with_body("echo hi", vec![], TestProvider::default());
    app.nav_mode = NavigationMode::Browse;
    app.preview_scroll = 9;
    let _ = app.handle_key(press(KeyCode::Char('x')));
    assert_eq!(app.preview_scroll, 0);
}

#[test]
fn browse_backspace_resets_preview_scroll() {
    let mut app = app_with_body("echo hi", vec![], TestProvider::default());
    app.nav_mode = NavigationMode::Browse;
    app.browse.input = "x".to_string();
    app.preview_scroll = 9;
    let _ = app.handle_key(press(KeyCode::Backspace));
    assert_eq!(app.preview_scroll, 0);
}

#[test]
fn browse_tab_resets_preview_scroll() {
    let file = SnippetFile {
        path: PathBuf::from("git/commits.md"),
        relative_path: PathBuf::from("git/commits.md"),
        frontmatter: crate::domain::Frontmatter::default(),
        snippets: vec![crate::domain::Snippet {
            id: crate::domain::SnippetId::new("git/commits.md", "slug"),
            name: "Log".to_string(),
            description: "desc".to_string(),
            body: "git log".to_string(),
            variables: vec![],
        }],
    };
    let index = SnippetIndex::from_files([file]);
    let frecency = FrecencyStore::new();
    let mut app = ExecutionApp::new(
        index,
        frecency,
        PathBuf::from("."),
        0,
        crate::config::SearchConfig::default(),
        crate::config::Theme::default(),
        TestProvider::default(),
    );
    app.nav_mode = NavigationMode::Browse;
    app.browse.input = "g".to_string();
    app.preview_scroll = 9;
    let _ = app.handle_key(press(KeyCode::Tab));
    assert_eq!(app.preview_scroll, 0);
}

#[test]
fn mode_toggle_resets_preview_scroll() {
    let mut app = app_with_body("echo hi", vec![], TestProvider::default());
    app.preview_scroll = 9;
    let _ = app.handle_key(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL));
    assert_eq!(app.preview_scroll, 0);
}

#[test]
fn browse_entering_directory_resets_preview_scroll() {
    let file = SnippetFile {
        path: PathBuf::from("git/commits.md"),
        relative_path: PathBuf::from("git/commits.md"),
        frontmatter: crate::domain::Frontmatter::default(),
        snippets: vec![crate::domain::Snippet {
            id: crate::domain::SnippetId::new("git/commits.md", "slug"),
            name: "Log".to_string(),
            description: "desc".to_string(),
            body: "git log".to_string(),
            variables: vec![],
        }],
    };
    let index = SnippetIndex::from_files([file]);
    let frecency = FrecencyStore::new();
    let mut app = ExecutionApp::new(
        index,
        frecency,
        PathBuf::from("."),
        0,
        crate::config::SearchConfig::default(),
        crate::config::Theme::default(),
        TestProvider::default(),
    );
    app.nav_mode = NavigationMode::Browse;
    app.preview_scroll = 9;
    let _ = app.handle_key(press(KeyCode::Enter));
    assert_eq!(app.browse.path, vec!["git".to_string()]);
    assert_eq!(app.preview_scroll, 0);
}

fn alt_enter() -> KeyEvent {
    KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

#[test]
fn alt_enter_inserts_newline_and_does_not_submit() {
    let variables = vec![Variable {
        name: "msg".to_string(),
        source: VariableSource::Free,
    }];
    let mut app = app_with_body("echo <@msg>", variables, TestProvider::default());
    // Enter the prompt screen.
    let _ = app.handle_key(press(KeyCode::Enter));
    let _ = app.handle_key(press(KeyCode::Char('a')));
    let _ = app.handle_key(alt_enter());
    let _ = app.handle_key(press(KeyCode::Char('b')));
    let outcome = completed(app.handle_key(press(KeyCode::Enter)));
    assert_eq!(outcome.command, "echo a\nb");
}

#[test]
fn ctrl_j_inserts_newline_in_prompt_input() {
    let variables = vec![Variable {
        name: "msg".to_string(),
        source: VariableSource::Free,
    }];
    let mut app = app_with_body("echo <@msg>", variables, TestProvider::default());
    let _ = app.handle_key(press(KeyCode::Enter));
    let _ = app.handle_key(press(KeyCode::Char('a')));
    let _ = app.handle_key(ctrl('j'));
    let _ = app.handle_key(press(KeyCode::Char('b')));
    let outcome = completed(app.handle_key(press(KeyCode::Enter)));
    assert_eq!(outcome.command, "echo a\nb");
}

#[test]
fn paste_into_prompt_preserves_newlines_as_single_value() {
    let variables = vec![Variable {
        name: "msg".to_string(),
        source: VariableSource::Free,
    }];
    let mut app = app_with_body("echo <@msg>", variables, TestProvider::default());
    let _ = app.handle_key(press(KeyCode::Enter));
    app.handle_paste("line1\nline2\nline3");
    let outcome = completed(app.handle_key(press(KeyCode::Enter)));
    assert_eq!(outcome.command, "echo line1\nline2\nline3");
}

#[test]
fn paste_normalizes_cr_and_crlf_to_lf() {
    // Bracketed paste on most terminals delivers \r (or \r\n) for newlines —
    // not \n. Without normalization the renderer's split('\n') leaves stray
    // \r in spans, which corrupts the inline viewport and desyncs the cursor.
    let variables = vec![Variable {
        name: "msg".to_string(),
        source: VariableSource::Free,
    }];
    let mut app = app_with_body("echo <@msg>", variables, TestProvider::default());
    let _ = app.handle_key(press(KeyCode::Enter));
    app.handle_paste("a\r\nb\rc");
    let outcome = completed(app.handle_key(press(KeyCode::Enter)));
    assert_eq!(outcome.command, "echo a\nb\nc");
}

#[test]
fn paste_strips_other_control_characters() {
    let variables = vec![Variable {
        name: "msg".to_string(),
        source: VariableSource::Free,
    }];
    let mut app = app_with_body("echo <@msg>", variables, TestProvider::default());
    let _ = app.handle_key(press(KeyCode::Enter));
    app.handle_paste("hi\x07\x1b[31mthere");
    let outcome = completed(app.handle_key(press(KeyCode::Enter)));
    assert_eq!(outcome.command, "echo hi[31mthere");
}

#[test]
fn paste_then_tab_cycles_to_next_variable_preserving_value() {
    let variables = vec![
        Variable {
            name: "first".to_string(),
            source: VariableSource::Free,
        },
        Variable {
            name: "second".to_string(),
            source: VariableSource::Free,
        },
    ];
    let mut app = app_with_body("<@first> | <@second>", variables, TestProvider::default());
    let _ = app.handle_key(press(KeyCode::Enter));
    app.handle_paste("a\nb");
    let _ = app.handle_key(press(KeyCode::Tab));
    let _ = app.handle_key(press(KeyCode::Char('z')));
    let outcome = completed(app.handle_key(press(KeyCode::Enter)));
    assert_eq!(outcome.command, "a\nb | z");
}

#[test]
fn paste_on_select_screen_is_dropped() {
    let variables = vec![Variable {
        name: "msg".to_string(),
        source: VariableSource::Free,
    }];
    let mut app = app_with_body("echo <@msg>", variables, TestProvider::default());
    // Still on the select screen — paste must not affect search query.
    app.handle_paste("multi\nline\npaste");
    assert_eq!(app.fuzzy.query, "");
}

#[test]
fn inline_default_with_embedded_newline_is_preserved() {
    let variables = vec![Variable {
        name: "block".to_string(),
        source: VariableSource::Default("line1\nline2".to_string()),
    }];
    let mut app = app_with_body(
        "cat <<EOF\n<@block:?ignored>\nEOF",
        variables,
        TestProvider::default(),
    );
    let _ = app.handle_key(press(KeyCode::Enter));
    let Screen::Prompt(prompt) = &app.screen else {
        panic!("expected prompt screen");
    };
    assert_eq!(prompt.input, "line1\nline2");
    let outcome = completed(app.handle_key(press(KeyCode::Enter)));
    assert_eq!(outcome.command, "cat <<EOF\nline1\nline2\nEOF");
}

#[test]
fn plain_enter_still_submits_after_keybinds_added() {
    let variables = vec![Variable {
        name: "msg".to_string(),
        source: VariableSource::Free,
    }];
    let mut app = app_with_body("echo <@msg>", variables, TestProvider::default());
    let _ = app.handle_key(press(KeyCode::Enter));
    let _ = app.handle_key(press(KeyCode::Char('h')));
    let _ = app.handle_key(press(KeyCode::Char('i')));
    let outcome = completed(app.handle_key(press(KeyCode::Enter)));
    assert_eq!(outcome.command, "echo hi");
}
