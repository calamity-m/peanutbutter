use crate::index::IndexedSnippet;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};
use ratatui::widgets::ListState;

const W_NAME: u32 = 30;
const W_TAG: u32 = 20;
const W_FM_NAME: u32 = 15;
const W_DESC: u32 = 10;
const W_PATH: u32 = 10;
const W_BODY: u32 = 8;

pub struct FuzzyScorer {
    matcher: Matcher,
    buf: Vec<char>,
}

impl FuzzyScorer {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
            buf: Vec::new(),
        }
    }

    pub fn score(&mut self, pattern: &Pattern, haystack: &str) -> Option<u32> {
        self.buf.clear();
        let hay = Utf32Str::new(haystack, &mut self.buf);
        pattern.score(hay, &mut self.matcher)
    }
}

impl Default for FuzzyScorer {
    fn default() -> Self {
        Self::new()
    }
}

pub fn build_pattern(query: &str) -> Pattern {
    Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart)
}

pub fn score_snippet(
    scorer: &mut FuzzyScorer,
    pattern: &Pattern,
    query_is_empty: bool,
    entry: &IndexedSnippet,
) -> Option<u32> {
    if query_is_empty {
        return Some(0);
    }

    let mut total: u32 = 0;
    let mut matched = false;

    let bump = |raw: Option<u32>, weight: u32, total: &mut u32, matched: &mut bool| {
        if let Some(v) = raw {
            *total = total.saturating_add(v.saturating_mul(weight));
            *matched = true;
        }
    };

    bump(
        scorer.score(pattern, entry.name()),
        W_NAME,
        &mut total,
        &mut matched,
    );
    bump(
        scorer.score(pattern, entry.body()),
        W_BODY,
        &mut total,
        &mut matched,
    );
    bump(
        scorer.score(pattern, entry.description()),
        W_DESC,
        &mut total,
        &mut matched,
    );
    let path = entry.relative_path_display();
    bump(
        scorer.score(pattern, &path),
        W_PATH,
        &mut total,
        &mut matched,
    );
    if let Some(name) = entry.frontmatter.name.as_deref() {
        bump(
            scorer.score(pattern, name),
            W_FM_NAME,
            &mut total,
            &mut matched,
        );
    }
    if let Some(desc) = entry.frontmatter.description.as_deref() {
        bump(
            scorer.score(pattern, desc),
            W_DESC,
            &mut total,
            &mut matched,
        );
    }
    for tag in entry.tags() {
        bump(scorer.score(pattern, tag), W_TAG, &mut total, &mut matched);
    }

    if matched { Some(total) } else { None }
}

/// Fuzzy search state. Holds the raw query string and a ratatui `ListState`
/// so Part 03 can render with `List::new(...).highlight_style(...)` and drive
/// selection directly. The query survives entering and leaving a snippet
/// detail view, so "backspace out of a snippet" returns the user to their
/// prior search exactly where they were.
#[derive(Debug, Default)]
pub struct FuzzyState {
    pub query: String,
    pub list: ListState,
}

impl FuzzyState {
    pub fn new() -> Self {
        let mut list = ListState::default();
        list.select(Some(0));
        Self {
            query: String::new(),
            list,
        }
    }

    pub fn set_query<S: Into<String>>(&mut self, query: S) {
        self.query = query.into();
        self.list.select(Some(0));
    }

    pub fn type_char(&mut self, c: char) {
        self.query.push(c);
        self.list.select(Some(0));
    }

    pub fn backspace(&mut self) -> bool {
        let changed = self.query.pop().is_some();
        if changed {
            self.list.select(Some(0));
        }
        changed
    }

    pub fn move_cursor(&mut self, delta: i32, result_len: usize) {
        if result_len == 0 {
            self.list.select(None);
            return;
        }
        let current = self.list.selected().unwrap_or(0) as i32;
        let next = (current + delta).clamp(0, result_len as i32 - 1);
        self.list.select(Some(next as usize));
    }

    pub fn selected(&self) -> Option<usize> {
        self.list.selected()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Frontmatter, Snippet, SnippetId};
    use std::path::PathBuf;

    fn entry(name: &str, body: &str, tags: &[&str], rel: &str) -> IndexedSnippet {
        IndexedSnippet {
            path: PathBuf::from(rel),
            snippet: Snippet {
                id: SnippetId::new(rel, "slug"),
                name: name.to_string(),
                description: String::new(),
                body: body.to_string(),
                variables: vec![],
            },
            relative_path: PathBuf::from(rel),
            frontmatter: Frontmatter {
                name: None,
                description: None,
                tags: tags.iter().map(|s| s.to_string()).collect(),
            },
        }
    }

    #[test]
    fn empty_query_matches_everything() {
        let mut scorer = FuzzyScorer::new();
        let pattern = build_pattern("");
        let e = entry("git log", "git log --oneline", &["git"], "git/log.md");
        assert_eq!(score_snippet(&mut scorer, &pattern, true, &e), Some(0));
    }

    #[test]
    fn name_beats_body_for_same_query() {
        let mut scorer = FuzzyScorer::new();
        let pattern = build_pattern("git");
        let name_match = entry("git log", "echo foo", &[], "a.md");
        let body_match = entry("zzz", "git log --oneline", &[], "b.md");
        let ns = score_snippet(&mut scorer, &pattern, false, &name_match).unwrap();
        let bs = score_snippet(&mut scorer, &pattern, false, &body_match).unwrap();
        assert!(ns > bs, "name score {ns} should beat body score {bs}");
    }

    #[test]
    fn non_matching_query_returns_none() {
        let mut scorer = FuzzyScorer::new();
        let pattern = build_pattern("xyzq");
        let e = entry("git log", "git log --oneline", &[], "a.md");
        assert!(score_snippet(&mut scorer, &pattern, false, &e).is_none());
    }

    #[test]
    fn tag_match_scores() {
        let mut scorer = FuzzyScorer::new();
        let pattern = build_pattern("docker");
        let e = entry("run it", "echo", &["docker", "compose"], "r.md");
        assert!(score_snippet(&mut scorer, &pattern, false, &e).is_some());
    }

    #[test]
    fn typing_resets_selection_to_top() {
        let mut state = FuzzyState::new();
        state.list.select(Some(5));
        state.type_char('a');
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn new_state_selects_first_result_by_default() {
        let state = FuzzyState::new();
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn move_cursor_clamps_to_result_range() {
        let mut state = FuzzyState::new();
        state.list.select(Some(0));
        state.move_cursor(10, 3);
        assert_eq!(state.selected(), Some(2));
        state.move_cursor(-100, 3);
        assert_eq!(state.selected(), Some(0));
    }
}
