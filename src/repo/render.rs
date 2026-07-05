//! Frame rendering for the `pb repo` TUI.

use crate::repo::app::{RepoApp, RepoGitState};
use crate::tui::chrome::Chrome;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

const FOOTER: &str =
    "↑/↓ select · s sync · p push · u pull · h hide/unhide · enter jump · r refresh · q quit";

pub(crate) fn draw(frame: &mut Frame, app: &RepoApp) {
    let area = frame.area();
    let title = format!("{} snippet repos", app.repos.len());
    let content = Chrome {
        theme: &app.theme,
        mode: "pb repo",
        title: &title,
        footer: FOOTER,
    }
    .render(area, frame.buffer_mut());
    if content.height == 0 {
        return;
    }

    // Bottom content row is reserved for the status line.
    let status_area = Rect {
        y: content.y + content.height - 1,
        height: 1,
        ..content
    };
    let list_area = Rect {
        height: content.height.saturating_sub(1),
        ..content
    };

    if app.repos.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "no git repositories found under the configured snippet roots",
                app.theme.placeholder,
            )),
            list_area,
        );
    } else {
        // Keep the selected row visible in a simple scrolling window.
        let visible = list_area.height as usize;
        let offset = app.selected.saturating_sub(visible.saturating_sub(1));
        let lines: Vec<Line> = app
            .repos
            .iter()
            .zip(app.git_states.iter())
            .enumerate()
            .skip(offset)
            .take(visible.max(1))
            .map(|(i, (repo, state))| repo_line(app, i, repo, state))
            .collect();
        frame.render_widget(Paragraph::new(lines), list_area);
    }

    if let Some(status) = &app.status {
        frame.render_widget(
            Paragraph::new(Span::styled(status.clone(), app.theme.chrome)),
            status_area,
        );
    }
}

fn repo_line<'a>(
    app: &'a RepoApp,
    index: usize,
    repo: &'a crate::repo::discover::SnippetRepo,
    state: &'a RepoGitState,
) -> Line<'a> {
    let selected = index == app.selected;
    let marker_style = if selected {
        app.theme.selected_marker
    } else {
        app.theme.chrome
    };
    let name_style = if selected {
        app.theme.selected_item
    } else {
        app.theme.emphasis
    };
    let mut spans = vec![
        Span::styled(if selected { "> " } else { "  " }, marker_style),
        Span::styled(repo.display.clone(), name_style),
    ];
    if repo.hidden {
        spans.push(Span::styled(" [hidden]", app.theme.placeholder));
    }
    spans.push(Span::raw("  "));
    match state {
        RepoGitState::Unknown => spans.push(Span::styled("…", app.theme.chrome)),
        RepoGitState::Summary(summary) => {
            spans.push(Span::styled(summary.describe(), app.theme.chrome))
        }
        RepoGitState::Error(err) => spans.push(Span::styled(err.clone(), app.theme.error)),
    }
    Line::from(spans)
}
