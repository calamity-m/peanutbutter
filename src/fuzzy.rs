use crate::config::FuzzyWeights;
use crate::index::IndexedSnippet;
use nucleo_matcher::pattern::{CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

pub struct FuzzyScorer {
    matcher: Matcher,
    buf: Vec<char>,
    indices_buf: Vec<u32>,
}

impl FuzzyScorer {
    pub fn new() -> Self {
        Self {
            matcher: Matcher::new(Config::DEFAULT),
            buf: Vec::new(),
            indices_buf: Vec::new(),
        }
    }

    pub fn score(&mut self, pattern: &Pattern, haystack: &str) -> Option<u32> {
        self.buf.clear();
        let hay = Utf32Str::new(haystack, &mut self.buf);
        pattern.score(hay, &mut self.matcher)
    }

    pub fn indices(&mut self, pattern: &Pattern, haystack: &str) -> Option<Vec<usize>> {
        self.buf.clear();
        self.indices_buf.clear();
        let hay = Utf32Str::new(haystack, &mut self.buf);
        pattern.indices(hay, &mut self.matcher, &mut self.indices_buf)?;
        self.indices_buf.sort_unstable();
        self.indices_buf.dedup();
        Some(self.indices_buf.iter().map(|idx| *idx as usize).collect())
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
    weights: &FuzzyWeights,
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
        weights.name,
        &mut total,
        &mut matched,
    );
    bump(
        scorer.score(pattern, entry.body()),
        weights.body,
        &mut total,
        &mut matched,
    );
    bump(
        scorer.score(pattern, entry.description()),
        weights.description,
        &mut total,
        &mut matched,
    );
    let path = entry.relative_path_display();
    bump(
        scorer.score(pattern, &path),
        weights.path,
        &mut total,
        &mut matched,
    );
    if let Some(name) = entry.frontmatter.name.as_deref() {
        bump(
            scorer.score(pattern, name),
            weights.frontmatter_name,
            &mut total,
            &mut matched,
        );
    }
    if let Some(desc) = entry.frontmatter.description.as_deref() {
        bump(
            scorer.score(pattern, desc),
            weights.description,
            &mut total,
            &mut matched,
        );
    }
    for tag in entry.tags() {
        bump(
            scorer.score(pattern, tag),
            weights.tag,
            &mut total,
            &mut matched,
        );
    }

    if matched { Some(total) } else { None }
}

/// Fuzzy search state. Holds the raw query string and the currently selected
/// result index. The query survives entering and leaving a snippet detail view,
/// so "backspace out of a snippet" returns the user to their prior search.
#[derive(Debug, Default)]
pub struct FuzzyState {
    pub query: String,
    pub cursor: usize, // byte offset into query
    pub selection: Option<usize>,
}

impl FuzzyState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            cursor: 0,
            selection: Some(0),
        }
    }

    pub fn set_query<S: Into<String>>(&mut self, query: S) {
        self.query = query.into();
        self.cursor = self.query.len();
        self.selection = Some(0);
    }

    pub fn type_char(&mut self, c: char) {
        self.query.insert(self.cursor, c);
        self.cursor += c.len_utf8();
        self.selection = Some(0);
    }

    pub fn backspace(&mut self) -> bool {
        if self.cursor == 0 {
            return false;
        }
        let prev = self.query[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
        self.query.remove(prev);
        self.cursor = prev;
        self.selection = Some(0);
        true
    }

    pub fn cursor_left(&mut self) {
        if self.cursor == 0 {
            return;
        }
        self.cursor = self.query[..self.cursor]
            .char_indices()
            .next_back()
            .map(|(i, _)| i)
            .unwrap_or(0);
    }

    pub fn cursor_right(&mut self) {
        if self.cursor >= self.query.len() {
            return;
        }
        let c = self.query[self.cursor..].chars().next().unwrap();
        self.cursor += c.len_utf8();
    }

    pub fn move_cursor(&mut self, delta: i32, result_len: usize) {
        if result_len == 0 {
            self.selection = None;
            return;
        }
        let current = self.selection.unwrap_or(0) as i32;
        let next = (current + delta).clamp(0, result_len as i32 - 1);
        self.selection = Some(next as usize);
    }

    pub fn selected(&self) -> Option<usize> {
        self.selection
    }

    /// Display-column offset of the cursor within the query (for rendering).
    pub fn cursor_col(&self) -> usize {
        self.query[..self.cursor].chars().count()
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
        assert_eq!(
            score_snippet(&mut scorer, &pattern, true, &e, &FuzzyWeights::default()),
            Some(0)
        );
    }

    #[test]
    fn name_beats_body_for_same_query() {
        let mut scorer = FuzzyScorer::new();
        let pattern = build_pattern("git");
        let name_match = entry("git log", "echo foo", &[], "a.md");
        let body_match = entry("zzz", "git log --oneline", &[], "b.md");
        let ns = score_snippet(
            &mut scorer,
            &pattern,
            false,
            &name_match,
            &FuzzyWeights::default(),
        )
        .unwrap();
        let bs = score_snippet(
            &mut scorer,
            &pattern,
            false,
            &body_match,
            &FuzzyWeights::default(),
        )
        .unwrap();
        assert!(ns > bs, "name score {ns} should beat body score {bs}");
    }

    #[test]
    fn non_matching_query_returns_none() {
        let mut scorer = FuzzyScorer::new();
        let pattern = build_pattern("xyzq");
        let e = entry("git log", "git log --oneline", &[], "a.md");
        assert!(
            score_snippet(&mut scorer, &pattern, false, &e, &FuzzyWeights::default()).is_none()
        );
    }

    #[test]
    fn tag_match_scores() {
        let mut scorer = FuzzyScorer::new();
        let pattern = build_pattern("docker");
        let e = entry("run it", "echo", &["docker", "compose"], "r.md");
        assert!(
            score_snippet(&mut scorer, &pattern, false, &e, &FuzzyWeights::default()).is_some()
        );
    }

    #[test]
    fn typing_resets_selection_to_top() {
        let mut state = FuzzyState::new();
        state.selection = Some(5);
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
        state.move_cursor(10, 3);
        assert_eq!(state.selected(), Some(2));
        state.move_cursor(-100, 3);
        assert_eq!(state.selected(), Some(0));
    }
}
