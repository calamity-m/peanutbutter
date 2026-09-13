use crate::config::FuzzyWeights;
use crate::index::IndexedSnippet;
use nucleo_matcher::pattern::{Atom, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// Thin wrapper around the `nucleo` fuzzy matcher that reuses its internal
/// buffers across calls. Construct once and pass mutably to scoring functions.
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

    /// Score one parsed query atom, preserving Nucleo's negation semantics.
    pub fn score_atom(&mut self, atom: &Atom, haystack: &str) -> Option<u32> {
        self.buf.clear();
        let hay = Utf32Str::new(haystack, &mut self.buf);
        atom.score(hay, &mut self.matcher).map(u32::from)
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

/// Compile a fuzzy query string into a reusable [`Pattern`].
///
/// Matching is case-insensitive and uses Unicode smart normalisation so that
/// e.g. accented characters are treated as their ASCII base.
pub fn build_pattern(query: &str) -> Pattern {
    Pattern::parse(query, CaseMatching::Ignore, Normalization::Smart)
}

/// Score a single snippet against a compiled pattern, summing weighted field
/// scores. Returns `None` if the query is non-empty and no field matched,
/// which lets callers filter out non-matching entries efficiently.
///
/// When `query_is_empty` is `true` the pattern is ignored and `Some(0)` is
/// returned for every entry so all snippets appear unfiltered.
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

    visit_fields(entry, weights, |text, weight| {
        if let Some(raw) = scorer.score(pattern, text) {
            total = total.saturating_add(raw.saturating_mul(weight));
            matched = true;
        }
    });

    if matched { Some(total) } else { None }
}

/// Sum each positive atom's best weighted field score; negatives must pass every field.
///
/// Taking the maximum after weighting prevents repeated metadata from stacking
/// weaker matches above stronger name matches. Default weights favor names, but
/// custom weights and the separately added frecency score can change ordering.
/// Unlike legacy accumulation, even single-atom scores can be lower.
/// Matching is independent of weight, so zero-weight fields still admit positive
/// terms and reject excluded terms. Negative-only patterns contribute zero.
pub fn score_snippet_cross_field(
    scorer: &mut FuzzyScorer,
    pattern: &Pattern,
    entry: &IndexedSnippet,
    weights: &FuzzyWeights,
) -> Option<u32> {
    let mut total = 0_u32;
    for atom in &pattern.atoms {
        let mut matched = false;
        let mut best = 0;
        let mut excluded = false;
        visit_fields(entry, weights, |text, weight| {
            let raw = scorer.score_atom(atom, text);
            if atom.negative {
                // A negative atom returns None precisely when its needle matches.
                excluded |= raw.is_none();
            } else if let Some(raw) = raw {
                matched = true;
                best = best.max(raw.saturating_mul(weight));
            }
        });
        if excluded || (!atom.negative && !matched) {
            return None;
        }
        total = total.saturating_add(best);
    }
    Some(total)
}

// Keep both modes on the same individual fields, without joining tags or adding
// the heading slug/language that are not part of free-text matching.
fn visit_fields(entry: &IndexedSnippet, weights: &FuzzyWeights, mut visit: impl FnMut(&str, u32)) {
    visit(entry.name(), weights.name);
    visit(entry.body(), weights.command);
    visit(entry.description(), weights.description);
    visit(&entry.relative_path_display(), weights.path);
    if let Some(name) = entry.frontmatter.name.as_deref() {
        visit(name, weights.frontmatter_name);
    }
    if let Some(description) = entry.frontmatter.description.as_deref() {
        visit(description, weights.description);
    }
    for tag in entry.tags() {
        visit(tag, weights.tag);
    }
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
                language: None,
            },
            relative_path: PathBuf::from(rel),
            frontmatter: Frontmatter {
                name: None,
                description: None,
                tags: tags.iter().map(|s| s.to_string()).collect(),
                variables: Default::default(),
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
    fn name_beats_command_for_same_query() {
        let mut scorer = FuzzyScorer::new();
        let pattern = build_pattern("git");
        let name_match = entry("git log", "echo foo", &[], "a.md");
        let command_match = entry("zzz", "git log --oneline", &[], "b.md");
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
            &command_match,
            &FuzzyWeights::default(),
        )
        .unwrap();
        assert!(ns > bs, "name score {ns} should beat command score {bs}");
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

    fn cross_score(query: &str, entry: &IndexedSnippet, weights: &FuzzyWeights) -> Option<u32> {
        score_snippet_cross_field(
            &mut FuzzyScorer::new(),
            &build_pattern(query),
            entry,
            weights,
        )
    }

    #[test]
    fn cross_field_covers_each_individual_field_and_excludes_from_it() {
        let mut e = entry("heading", "command", &["firsttag", "secondtag"], "path.md");
        e.snippet.description = "prose".into();
        e.frontmatter.name = Some("collection".into());
        e.frontmatter.description = Some("summary".into());
        let weights = FuzzyWeights::default();
        assert!(
            cross_score(
                "heading command firsttag secondtag path prose collection summary",
                &e,
                &weights,
            )
            .is_some()
        );
        for field in [
            "heading",
            "command",
            "firsttag",
            "secondtag",
            "path",
            "prose",
            "collection",
            "summary",
        ] {
            assert_eq!(
                cross_score(&format!("!{field}"), &e, &weights),
                None,
                "{field}"
            );
        }
        assert_eq!(cross_score("missingneedle", &e, &weights), None);
        // One atom may not bridge separate tags, even though two atoms can.
        assert_eq!(cross_score(r"firsttag\ secondtag", &e, &weights), None);
        e.snippet.language = Some("languageonly".into());
        assert_eq!(cross_score("languageonly", &e, &weights), None);
        assert_eq!(cross_score("slug", &e, &weights), None);
    }

    #[test]
    fn cross_field_uses_best_weighted_field_per_atom() {
        let mut e = entry("needle", "needle", &["needle", "needle"], "needle");
        e.snippet.description = "needle".into();
        e.frontmatter.name = Some("needle".into());
        e.frontmatter.description = Some("needle".into());
        let weights = FuzzyWeights::default();
        let mut scorer = FuzzyScorer::new();
        let pattern = build_pattern("needle");
        let raw = scorer.score(&pattern, "needle").unwrap();
        let expected = raw
            * (weights.name
                + weights.command
                + 2 * weights.description
                + weights.frontmatter_name
                + weights.path
                + 2 * weights.tag);
        // Legacy scoring still accumulates every field, even for one atom.
        assert_eq!(
            score_snippet(&mut scorer, &pattern, false, &e, &weights),
            Some(expected)
        );
        assert_eq!(
            cross_score("needle", &e, &weights),
            Some(raw * weights.name)
        );
        assert_eq!(
            cross_score("needle needle", &e, &weights),
            Some(2 * raw * weights.name)
        );
        let weights = FuzzyWeights {
            command: 100,
            ..weights
        };
        assert_eq!(
            cross_score("needle", &e, &weights),
            Some(raw * weights.command)
        );
    }

    #[test]
    fn cross_field_compares_scores_after_weighting() {
        let weights = FuzzyWeights::default();
        let mut scorer = FuzzyScorer::new();
        let pattern = build_pattern("docker");
        let tag_raw = scorer.score(&pattern, "docker").unwrap();
        for (name, name_wins) in [("dxxoxxcxxkxxexxr", false), ("d___o___c___k___e___r", true)] {
            let e = entry(name, "echo", &["docker"], "plain.md");
            let name_raw = scorer.score(&pattern, name).unwrap();
            assert!(name_raw < tag_raw);
            let name_score = name_raw * weights.name;
            let tag_score = tag_raw * weights.tag;
            assert_eq!(name_score > tag_score, name_wins);
            assert_eq!(
                cross_score("docker", &e, &weights),
                Some(if name_wins { name_score } else { tag_score })
            );
        }
    }

    #[test]
    fn cross_field_duplicate_tags_do_not_inflate_scores() {
        let mut e = entry(
            "compose up",
            "docker compose up -d && docker ps",
            &["docker"],
            "docker/compose.md",
        );
        let weights = FuzzyWeights::default();
        let before = cross_score("docker ps", &e, &weights).unwrap();
        e.frontmatter.tags.extend(vec!["docker".to_string(); 100]);
        assert_eq!(cross_score("docker ps", &e, &weights), Some(before));
    }

    #[test]
    fn cross_field_zero_weights_still_match_and_exclude() {
        let weights = FuzzyWeights {
            name: 0,
            command: 0,
            description: 0,
            frontmatter_name: 0,
            path: 0,
            tag: 0,
        };
        let e = entry("kitchen", "eza", &["docker"], "p.md");
        assert_eq!(cross_score("kitchen eza", &e, &weights), Some(0));
        assert_eq!(cross_score("kitchen !docker", &e, &weights), None);
        assert_eq!(cross_score("!absent", &e, &weights), Some(0));
        assert_eq!(cross_score("", &e, &weights), Some(0));
    }

    #[test]
    fn cross_field_weighted_scores_saturate() {
        let e = entry("needle", "needle", &[], "needle");
        let weights = FuzzyWeights {
            name: u32::MAX,
            ..FuzzyWeights::default()
        };
        assert_eq!(cross_score("needle needle", &e, &weights), Some(u32::MAX));
        // Each weighted atom fits, but summing two atoms must saturate too.
        let raw = FuzzyScorer::new()
            .score(&build_pattern("needle"), "needle")
            .unwrap();
        let weights = FuzzyWeights {
            name: u32::MAX / raw / 2 + 1,
            ..FuzzyWeights::default()
        };
        let single = cross_score("needle", &e, &weights).unwrap();
        assert!(single < u32::MAX);
        assert!(u64::from(single) * 2 > u64::from(u32::MAX));
        assert_eq!(cross_score("needle needle", &e, &weights), Some(u32::MAX));
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
