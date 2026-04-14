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

fn app_with_body(
    body: &str,
    variables: Vec<Variable>,
    provider: TestProvider,
) -> ExecutionApp<TestProvider> {
    let index = SnippetIndex::from_files([snippet_file("x.md", "Demo", body, variables)]);
    let frecency = FrecencyStore::new();
    ExecutionApp::new(index, frecency, PathBuf::from("."), 0, provider)
}

fn press(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn completed(event: AppEvent) -> ExecutionOutcome {
    match event {
        AppEvent::Completed(outcome) => outcome,
        AppEvent::Continue => panic!("expected completed event, got continue"),
        AppEvent::Cancelled => panic!("expected completed event, got cancelled"),
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

#[test]
fn render_command_text_highlights_active_value() {
    let mut values = BTreeMap::new();
    values.insert("file".to_string(), "Cargo.toml".to_string());
    let rendered = render_command_text("cat <@file>", &values, Some("file"));
    assert_eq!(line_text(&rendered.lines[0]), "cat Cargo.toml");
    assert_eq!(rendered.lines[0].spans[4].style, active_prompt_style());
}

#[test]
fn render_command_text_highlights_active_placeholder_and_dims_others() {
    let values = BTreeMap::new();
    let rendered = render_command_text("echo <@missing> <@later>", &values, Some("missing"));
    assert_eq!(line_text(&rendered.lines[0]), "echo <@missing> <@later>");
    assert_eq!(rendered.lines[0].spans[5].style, active_prompt_style());
    assert_eq!(rendered.lines[0].spans[7].style, placeholder_prompt_style());
}

#[test]
fn compact_viewport_height_respects_user_cap() {
    assert_eq!(compact_viewport_height(60, 12), 12);
    assert_eq!(compact_viewport_height(24, 12), 12);
    assert_eq!(compact_viewport_height(9, 12), 12);
    assert_eq!(compact_viewport_height(60, 4), 4);
}

#[test]
fn compact_viewport_height_enforces_20_row_minimum() {
    assert_eq!(compact_viewport_height(60, 20), 20);
    assert_eq!(compact_viewport_height(24, 20), 20);
    assert_eq!(compact_viewport_height(9, 20), 20);
    assert_eq!(compact_viewport_height(90, 40), 30);
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
        TestProvider::default(),
    );
    app.nav_mode = NavigationMode::Browse;
    app.browse.path = vec!["git".to_string()];
    app.browse.input = String::new();
    app.browse.list.select(Some(0));

    let _ = app.handle_key(press(KeyCode::Enter));
    assert!(matches!(app.screen, Screen::Prompt(_)));

    let _ = app.handle_key(press(KeyCode::Esc));
    assert!(matches!(app.screen, Screen::Select));
    assert_eq!(app.browse.path, vec!["git".to_string()]);
    assert_eq!(app.browse.input, "");
    assert_eq!(app.browse.list.selected(), Some(0));
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
        TestProvider::default(),
    );
    app.nav_mode = NavigationMode::Browse;
    app.preview_scroll = 9;
    let _ = app.handle_key(press(KeyCode::Enter));
    assert_eq!(app.browse.path, vec!["git".to_string()]);
    assert_eq!(app.preview_scroll, 0);
}
