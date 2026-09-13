use crate::config::SearchConfig;
use crate::frecency::FrecencyStore;
use crate::fuzzy::{FuzzyScorer, build_pattern, score_snippet, score_snippet_cross_field};
use crate::index::{IndexedSnippet, SnippetIndex};
use std::path::Path;

/// A single result row. `fuzzy` is `None` when the query is empty so the
/// caller can render "everything, ordered by frecency" without a special case.
#[derive(Debug, Clone)]
pub struct SearchHit<'a> {
    pub snippet: &'a IndexedSnippet,
    pub fuzzy: Option<u32>,
    pub frecency: f64,
    pub combined: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueryField {
    Name,
    Path,
    Tag,
    Body,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FieldTerm {
    field: QueryField,
    value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ParsedQuery {
    free_text: String,
    terms: Vec<FieldTerm>,
}

impl ParsedQuery {
    fn parse(raw: &str) -> Self {
        let mut parser = QueryParser::new(raw);
        let mut free_tokens = Vec::new();
        let mut terms = Vec::new();

        while let Some(token) = parser.next_token() {
            if let Some(term) = parse_field_term(&token) {
                terms.push(term);
            } else {
                free_tokens.push(token.raw);
            }
        }

        Self {
            free_text: free_tokens.join(" "),
            terms,
        }
    }

    fn is_empty(&self) -> bool {
        self.free_text.is_empty() && self.terms.is_empty()
    }
}

/// One parsed query pattern used by renderers for fuzzy highlighting.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HighlightTerm {
    /// `None` means free-text highlighting across every searchable field.
    pub(crate) field: Option<QueryField>,
    /// Nucleo pattern text to highlight.
    pub(crate) value: String,
}

/// Return parsed pattern text renderers should use for fuzzy highlighting.
pub(crate) fn highlight_terms(query: &str) -> Vec<HighlightTerm> {
    let parsed = ParsedQuery::parse(query.trim());
    if parsed.is_empty() {
        return Vec::new();
    }

    let mut terms = Vec::new();
    if !parsed.free_text.is_empty() {
        terms.push(HighlightTerm {
            field: None,
            value: parsed.free_text,
        });
    }
    for term in parsed.terms {
        terms.push(HighlightTerm {
            field: Some(term.field),
            value: term.value,
        });
    }
    terms
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct QueryToken {
    raw: String,
    value: String,
}

struct QueryParser<'a> {
    raw: &'a str,
    pos: usize,
}

impl<'a> QueryParser<'a> {
    fn new(raw: &'a str) -> Self {
        Self { raw, pos: 0 }
    }

    fn next_token(&mut self) -> Option<QueryToken> {
        self.skip_whitespace();
        if self.pos >= self.raw.len() {
            return None;
        }

        let start = self.pos;
        let quote_start = self.field_value_quote_start(start);
        if let Some((quote_pos, quote)) = quote_start
            && let Some(end) = self.find_closing_quote(quote_pos + quote.len_utf8(), quote)
        {
            self.pos = end;
            let raw = self.raw[start..end].to_string();
            let value_start = quote_pos + quote.len_utf8();
            let value_end = end - quote.len_utf8();
            let mut value = self.raw[start..quote_pos].to_string();
            value.push_str(&self.raw[value_start..value_end]);
            return Some(QueryToken { raw, value });
        }

        while self.pos < self.raw.len() {
            let c = self.raw[self.pos..].chars().next().unwrap();
            if c.is_whitespace() {
                break;
            }
            self.pos += c.len_utf8();
        }

        let raw = self.raw[start..self.pos].to_string();
        Some(QueryToken {
            value: raw.clone(),
            raw,
        })
    }

    fn skip_whitespace(&mut self) {
        while self.pos < self.raw.len() {
            let c = self.raw[self.pos..].chars().next().unwrap();
            if !c.is_whitespace() {
                break;
            }
            self.pos += c.len_utf8();
        }
    }

    fn field_value_quote_start(&self, start: usize) -> Option<(usize, char)> {
        let rest = &self.raw[start..];
        let colon = rest.find(':')?;
        let quote_pos = start + colon + 1;
        let quote = self.raw[quote_pos..].chars().next()?;
        matches!(quote, '"' | '\'').then_some((quote_pos, quote))
    }

    fn find_closing_quote(&self, mut pos: usize, quote: char) -> Option<usize> {
        while pos < self.raw.len() {
            let c = self.raw[pos..].chars().next().unwrap();
            pos += c.len_utf8();
            if c == quote {
                return Some(pos);
            }
        }
        None
    }
}

fn parse_field_term(token: &QueryToken) -> Option<FieldTerm> {
    for (prefix, field) in [
        ("name:", QueryField::Name),
        ("path:", QueryField::Path),
        ("tag:", QueryField::Tag),
        ("snippet:", QueryField::Body),
        ("command:", QueryField::Body),
        ("body:", QueryField::Body),
    ] {
        if let Some(value) = token.value.strip_prefix(prefix)
            && !value.is_empty()
        {
            return Some(FieldTerm {
                field,
                value: value.to_string(),
            });
        }
    }
    None
}

/// Rank every snippet in `index` for the given `query` and `cwd`.
///
/// 1. Each snippet is scored with free-text fuzzy matching over weighted
///    fields plus any field-scoped query operators. Non-matches are dropped.
/// 2. A frecency score is computed via [`FrecencyStore::score`].
/// 3. The combined score is `fuzzy + frecency * config.frecency_weight`.
/// 4. Results are sorted descending by combined score, with name as tiebreaker.
pub fn rank<'a>(
    index: &'a SnippetIndex,
    query: &str,
    frecency: &FrecencyStore,
    cwd: &Path,
    now: u64,
    config: &SearchConfig,
) -> Vec<SearchHit<'a>> {
    let mut scorer = FuzzyScorer::new();
    let parsed = ParsedQuery::parse(query.trim());
    let empty = parsed.is_empty();
    let free_pattern = build_pattern(&parsed.free_text);

    let mut hits: Vec<SearchHit<'a>> = index
        .iter()
        .filter_map(|entry| {
            let fuzzy = score_query(&mut scorer, &parsed, &free_pattern, empty, entry, config)?;
            let frec = frecency.score(entry.id(), cwd, now, &config.frecency);
            let combined = fuzzy as f64 + frec * config.frecency_weight;
            Some(SearchHit {
                snippet: entry,
                fuzzy: if empty { None } else { Some(fuzzy) },
                frecency: frec,
                combined,
            })
        })
        .collect();

    hits.sort_by(|a, b| {
        b.combined
            .partial_cmp(&a.combined)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.snippet.name().cmp(b.snippet.name()))
    });
    hits
}

fn score_query(
    scorer: &mut FuzzyScorer,
    parsed: &ParsedQuery,
    free_pattern: &nucleo_matcher::pattern::Pattern,
    query_is_empty: bool,
    entry: &IndexedSnippet,
    config: &SearchConfig,
) -> Option<u32> {
    if query_is_empty {
        return Some(0);
    }

    let mut total: u32 = 0;
    if !parsed.free_text.is_empty() {
        total = if config.cross_field_matching {
            score_snippet_cross_field(scorer, free_pattern, entry, &config.fuzzy)?
        } else {
            score_snippet(scorer, free_pattern, false, entry, &config.fuzzy)?
        };
    }

    for term in &parsed.terms {
        total = total.saturating_add(score_field_term(scorer, term, entry, config)?);
    }

    Some(total)
}

fn score_field_term(
    scorer: &mut FuzzyScorer,
    term: &FieldTerm,
    entry: &IndexedSnippet,
    config: &SearchConfig,
) -> Option<u32> {
    let pattern = build_pattern(&term.value);
    match term.field {
        QueryField::Name => best_score(
            scorer,
            &pattern,
            [entry.name(), snippet_heading_slug(entry)].into_iter(),
        )
        .map(|score| score.saturating_mul(config.fuzzy.name)),
        QueryField::Path => {
            let path = entry.relative_path_display();
            scorer
                .score(&pattern, &path)
                .map(|score| score.saturating_mul(config.fuzzy.path))
        }
        QueryField::Tag => best_score(
            scorer,
            &pattern,
            entry.tags().iter().map(std::string::String::as_str),
        )
        .map(|score| score.saturating_mul(config.fuzzy.tag)),
        QueryField::Body => scorer
            .score(&pattern, entry.body())
            .map(|score| score.saturating_mul(config.fuzzy.command)),
    }
}

fn best_score<'a>(
    scorer: &mut FuzzyScorer,
    pattern: &nucleo_matcher::pattern::Pattern,
    haystacks: impl Iterator<Item = &'a str>,
) -> Option<u32> {
    haystacks
        .filter_map(|haystack| scorer.score(pattern, haystack))
        .max()
}

fn snippet_heading_slug(entry: &IndexedSnippet) -> &str {
    entry
        .id()
        .as_str()
        .split_once('#')
        .map_or("", |(_, slug)| slug)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SearchConfig;
    use crate::domain::{Frontmatter, Snippet, SnippetFile, SnippetId};
    use crate::index::SnippetIndex;
    use std::path::PathBuf;

    fn make_file(rel: &str, heading: &str, body: &str, slug: &str) -> SnippetFile {
        make_tagged_file(rel, heading, body, slug, &[])
    }

    fn make_tagged_file(
        rel: &str,
        heading: &str,
        body: &str,
        slug: &str,
        tags: &[&str],
    ) -> SnippetFile {
        SnippetFile {
            path: PathBuf::from(rel),
            relative_path: PathBuf::from(rel),
            frontmatter: Frontmatter {
                tags: tags.iter().map(|tag| tag.to_string()).collect(),
                ..Default::default()
            },
            snippets: vec![Snippet {
                id: SnippetId::new(rel, slug),
                name: heading.to_string(),
                description: String::new(),
                body: body.to_string(),
                variables: vec![],
                language: None,
            }],
        }
    }

    fn operator_index() -> SnippetIndex {
        SnippetIndex::from_files([
            make_tagged_file(
                "ops/docker.md",
                "ship service",
                "kubectl logs deployment api",
                "ship-service",
                &["docker", "compose"],
            ),
            make_tagged_file(
                "guides/search.md",
                "search google",
                "open browser and search google exactly",
                "search-google",
                &["web"],
            ),
            make_tagged_file(
                "owners/calam.md",
                "ownership note",
                "owner:calam deploy handoff",
                "ownership-note",
                &["meta"],
            ),
            make_tagged_file(
                "plain/docker-body.md",
                "body mention",
                "docker appears only in the body",
                "body-mention",
                &["misc"],
            ),
            make_tagged_file(
                "foo/bar.md",
                "path only",
                "echo path",
                "path-only",
                &["misc"],
            ),
            make_tagged_file(
                "name-only.md",
                "foo command",
                "echo name",
                "foo-command",
                &["misc"],
            ),
        ])
    }

    fn ranked_ids(index: &SnippetIndex, query: &str) -> Vec<String> {
        rank(
            index,
            query,
            &FrecencyStore::new(),
            Path::new("/tmp"),
            0,
            &SearchConfig::default(),
        )
        .into_iter()
        .map(|hit| hit.snippet.id().as_str().to_string())
        .collect()
    }

    fn rank_with_config<'a>(
        index: &'a SnippetIndex,
        query: &str,
        config: &SearchConfig,
    ) -> Vec<SearchHit<'a>> {
        rank(
            index,
            query,
            &FrecencyStore::new(),
            Path::new("/tmp"),
            0,
            config,
        )
    }

    fn tiny_index() -> SnippetIndex {
        SnippetIndex::from_files([
            make_file("git/log.md", "git log pretty", "git log --oneline", "a"),
            make_file("docker/run.md", "docker run alpine", "docker run", "b"),
            make_file("misc/echo.md", "echo hello", "echo hello", "c"),
        ])
    }

    #[test]
    fn empty_query_returns_every_snippet() {
        let index = tiny_index();
        let store = FrecencyStore::new();
        let hits = rank(
            &index,
            "",
            &store,
            Path::new("/tmp"),
            0,
            &SearchConfig::default(),
        );
        assert_eq!(hits.len(), 3);
        for hit in &hits {
            assert!(hit.fuzzy.is_none());
        }
    }

    #[test]
    fn query_ranks_name_match_above_body_match() {
        let index = tiny_index();
        let store = FrecencyStore::new();
        let hits = rank(
            &index,
            "git",
            &store,
            Path::new("/tmp"),
            0,
            &SearchConfig::default(),
        );
        assert!(!hits.is_empty());
        assert!(hits[0].snippet.name().contains("git"));
    }

    #[test]
    fn frecency_breaks_ties_between_equivalent_fuzzy_matches() {
        let index = tiny_index();
        let mut store = FrecencyStore::new();
        // Give the "docker" entry a recent local-cwd usage so it outranks
        // "git" even though both share no query text.
        store.record(
            SnippetId::new("docker/run.md", "b"),
            PathBuf::from("/repo"),
            1000,
        );
        let hits = rank(
            &index,
            "",
            &store,
            Path::new("/repo"),
            1000,
            &SearchConfig::default(),
        );
        assert_eq!(hits[0].snippet.id().as_str(), "docker/run.md#b");
    }

    #[test]
    fn cross_field_matching_is_opt_in_and_requires_every_positive_term() {
        let index = SnippetIndex::from_files([make_tagged_file(
            "files/eza.md",
            "kitchen sink",
            "eza --all",
            "kitchen-sink",
            &["eza", "tools"],
        )]);
        let mut config = SearchConfig::default();
        for query in ["kitchen eza", "eza kitchen"] {
            config.cross_field_matching = false;
            assert!(rank_with_config(&index, query, &config).is_empty());
            config.cross_field_matching = true;
            let hits = rank_with_config(&index, query, &config);
            assert_eq!(hits.len(), 1);
            assert_eq!(hits[0].snippet.id().as_str(), "files/eza.md#kitchen-sink");
            assert!(hits[0].fuzzy.unwrap() > 0);
        }
        let forward = rank_with_config(&index, "kitchen eza", &config);
        let reverse = rank_with_config(&index, "eza kitchen", &config);
        assert_eq!(forward[0].fuzzy, reverse[0].fuzzy);
        for query in [
            "kitchen eza missing",
            "kitchen eza tag:missing",
            "kitchen eza name:eza",
            "kitchen eza body:kitchen",
        ] {
            assert!(
                rank_with_config(&index, query, &config).is_empty(),
                "{query}"
            );
        }
        let mixed = rank_with_config(&index, "kitchen eza tag:tools", &config);
        let scoped = rank_with_config(&index, "tag:tools", &config);
        assert_eq!(mixed.len(), 1);
        assert_eq!(
            mixed[0].fuzzy,
            Some(forward[0].fuzzy.unwrap() + scoped[0].fuzzy.unwrap())
        );
    }

    #[test]
    fn cross_field_scores_choose_best_weighted_field_for_each_atom() {
        let mut file = make_tagged_file(
            "eza.md",
            "kitchen",
            "eza",
            "all-fields",
            &["kitchen", "eza"],
        );
        file.snippets[0].description = "kitchen".to_string();
        file.frontmatter.name = Some("eza".to_string());
        file.frontmatter.description = Some("kitchen".to_string());
        let index = SnippetIndex::from_files([
            file,
            make_file("other.md", "kitchen eza", "echo", "name-only"),
        ]);
        let config = SearchConfig {
            cross_field_matching: true,
            fuzzy: crate::config::FuzzyWeights {
                name: 3,
                command: 5,
                description: 7,
                path: 11,
                frontmatter_name: 13,
                tag: 17,
            },
            ..SearchConfig::default()
        };
        let mut scorer = FuzzyScorer::new();
        let mut expected = 0;
        for (query, text, weight) in [
            ("kitchen", "kitchen", config.fuzzy.tag),
            ("eza", "eza", config.fuzzy.tag),
        ] {
            expected += scorer.score(&build_pattern(query), text).unwrap() * weight;
        }
        let hits = rank_with_config(&index, "kitchen eza", &config);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].snippet.id().as_str(), "eza.md#all-fields");
        assert_eq!(hits[0].fuzzy, Some(expected));
        assert_eq!(hits[0].combined, f64::from(expected));
        assert_eq!(hits[1].snippet.id().as_str(), "other.md#name-only");
        assert!(hits[0].fuzzy > hits[1].fuzzy);
    }

    #[test]
    fn cross_field_default_weights_favor_exact_names_over_scattered_matches() {
        let compose = make_tagged_file(
            "docker/compose.md",
            "compose up",
            "docker compose up -d && docker ps",
            "compose-up",
            &["docker"],
        );
        let mut described_compose = compose.clone();
        described_compose.frontmatter.description = Some("docker ps".into());
        let mut history = make_tagged_file(
            "git/history.md",
            "inspect history",
            "git status && git log --oneline",
            "history",
            &["git", "log"],
        );
        history.frontmatter.name = Some("git tools".into());
        history.frontmatter.description = Some("git log".into());
        history.snippets[0].description = "Inspect git log history".into();
        let mut kitchen = make_tagged_file(
            "files/eza.md",
            "kitchen sink",
            "eza --long",
            "kitchen",
            &["eza"],
        );
        kitchen.snippets[0].description = "kitchen utilities".into();
        let config = SearchConfig {
            cross_field_matching: true,
            ..SearchConfig::default()
        };
        for (query, body, other, exact_score, other_score) in [
            ("docker ps", "docker ps -a", described_compose, 6840, 4190),
            ("docker ps", "docker ps -a", compose, 6840, 4190),
            ("git log", "git log --oneline", history, 5280, 3520),
            ("kitchen eza", "echo ready", kitchen, 8400, 7520),
        ] {
            let index =
                SnippetIndex::from_files([other, make_file("plain.md", query, body, "exact")]);
            for equal_history in [false, true] {
                let mut store = FrecencyStore::new();
                if equal_history {
                    for entry in index.iter() {
                        store.record(entry.id().clone(), PathBuf::from("/tmp"), 1000);
                    }
                }
                let hits = rank(&index, query, &store, Path::new("/tmp"), 1000, &config);
                assert_eq!(hits.len(), 2, "{query}");
                assert_eq!(hits[0].snippet.id().as_str(), "plain.md#exact", "{query}");
                assert_eq!(hits[0].fuzzy, Some(exact_score), "{query}");
                assert_eq!(hits[1].fuzzy, Some(other_score), "{query}");
                assert_eq!(hits[0].frecency, hits[1].frecency);
                assert!(hits[0].combined > hits[1].combined, "{query}");
            }
        }
    }

    #[test]
    fn cross_field_single_atom_uses_best_field_while_legacy_still_accumulates() {
        let index =
            SnippetIndex::from_files([make_file("plain.md", "docker ps", "docker ps -a", "exact")]);
        let mut config = SearchConfig::default();
        let legacy = rank_with_config(&index, "docker", &config);
        assert_eq!(legacy[0].fuzzy, Some(6308));
        config.cross_field_matching = true;
        let hits = rank_with_config(&index, "docker", &config);
        assert_eq!(hits[0].fuzzy, Some(4980));
    }

    #[test]
    fn cross_field_frecency_can_override_fuzzy_ranking_but_not_exclusions() {
        let index = SnippetIndex::from_files([
            make_file("plain.md", "docker ps", "docker ps -a", "exact"),
            make_tagged_file(
                "docker/compose.md",
                "compose up",
                "docker compose up -d && docker ps",
                "compose-up",
                &["docker"],
            ),
        ]);
        let config = SearchConfig {
            cross_field_matching: true,
            ..SearchConfig::default()
        };
        let mut store = FrecencyStore::new();
        // One recent use should not erase this fuzzy gap; repeated use should.
        for uses in 1..=6 {
            store.record(
                SnippetId::new("docker/compose.md", "compose-up"),
                PathBuf::from("/tmp"),
                1000,
            );
            if uses == 1 || uses == 6 {
                let hits = rank(
                    &index,
                    "docker ps",
                    &store,
                    Path::new("/tmp"),
                    1000,
                    &config,
                );
                assert_eq!(
                    hits[0].snippet.name(),
                    if uses == 1 { "docker ps" } else { "compose up" }
                );
                for hit in hits {
                    assert_eq!(
                        hit.combined,
                        f64::from(hit.fuzzy.unwrap()) + hit.frecency * config.frecency_weight
                    );
                }
            }
        }
        let stale_now = 1000 + (10.0 * config.frecency.half_life_days * 86400.0) as u64;
        let stale = rank(
            &index,
            "docker ps",
            &store,
            Path::new("/tmp"),
            stale_now,
            &config,
        );
        assert_eq!(stale[0].snippet.name(), "docker ps");
        let excluded = rank(
            &index,
            "docker ps !compose",
            &store,
            Path::new("/tmp"),
            1000,
            &config,
        );
        assert_eq!(excluded.len(), 1);
        assert_eq!(excluded[0].snippet.name(), "docker ps");
        let config = SearchConfig {
            frecency_weight: 0.0,
            ..config
        };
        let fuzzy_only = rank(
            &index,
            "docker ps",
            &store,
            Path::new("/tmp"),
            1000,
            &config,
        );
        assert_eq!(fuzzy_only[0].snippet.name(), "docker ps");
    }

    #[test]
    fn cross_field_near_tie_can_be_overridden_by_recent_or_old_frequent_use() {
        let mut kitchen = make_tagged_file(
            "files/eza.md",
            "kitchen sink",
            "eza --long",
            "kitchen",
            &["eza"],
        );
        kitchen.snippets[0].description = "kitchen utilities".into();
        let index = SnippetIndex::from_files([
            make_file("plain.md", "kitchen eza", "echo ready", "exact"),
            kitchen,
        ]);
        let config = SearchConfig {
            cross_field_matching: true,
            ..SearchConfig::default()
        };
        // A close fuzzy result needs less history to win. The frequency bonus
        // does not decay, so enough old usage can still outweigh an exact name.
        for (uses, age_half_lives, expected_name) in [
            (0, 0, "kitchen eza"),
            (1, 0, "kitchen eza"),
            (2, 0, "kitchen sink"),
            (2, 100, "kitchen eza"),
            (33, 100, "kitchen sink"),
        ] {
            let mut store = FrecencyStore::new();
            for _ in 0..uses {
                store.record(
                    SnippetId::new("files/eza.md", "kitchen"),
                    PathBuf::from("/tmp"),
                    1000,
                );
            }
            let now = 1000
                + (f64::from(age_half_lives) * config.frecency.half_life_days * 86400.0) as u64;
            let hits = rank(
                &index,
                "kitchen eza",
                &store,
                Path::new("/tmp"),
                now,
                &config,
            );
            assert_eq!(
                hits[0].snippet.name(),
                expected_name,
                "uses={uses}, age_half_lives={age_half_lives}"
            );
        }
    }

    #[test]
    fn disabled_multiword_scores_and_order_match_direct_legacy_scoring() {
        let index = SnippetIndex::from_files([
            make_tagged_file(
                "a.md",
                "kitchen eza",
                "kitchen eza",
                "both",
                &["kitchen eza"],
            ),
            make_file("b.md", "kitchen eza", "echo", "name"),
            make_file("eza.md", "kitchen sink", "eza", "split"),
            make_file("c.md", "unrelated", "echo", "missing"),
        ]);
        let config = SearchConfig::default();
        assert!(!config.cross_field_matching);
        for query in ["kitchen eza", "eza kitchen", "kitchen eza !docker"] {
            let pattern = build_pattern(query);
            let mut scorer = FuzzyScorer::new();
            let mut expected: Vec<_> = index
                .iter()
                .filter_map(|entry| {
                    score_snippet(&mut scorer, &pattern, false, entry, &config.fuzzy)
                        .map(|score| (entry.id().as_str(), score))
                })
                .collect();
            expected.sort_by_key(|(_, score)| std::cmp::Reverse(*score));
            assert_eq!(expected.len(), 2);
            assert_eq!(expected[0].0, "a.md#both");
            assert_eq!(expected[1].0, "b.md#name");
            let hits = rank_with_config(&index, query, &config);
            let actual: Vec<_> = hits
                .iter()
                .map(|hit| {
                    assert_eq!(hit.combined, f64::from(hit.fuzzy.unwrap()));
                    (hit.snippet.id().as_str(), hit.fuzzy.unwrap())
                })
                .collect();
            assert_eq!(actual, expected, "{query}");
        }
    }

    #[test]
    fn negative_only_queries_exclude_globally_and_keep_zero_score_ranking() {
        let index = SnippetIndex::from_files([
            make_tagged_file("a.md", "alpha", "echo", "a", &["docker"]),
            make_file("b.md", "bravo", "echo", "b"),
            make_file("c.md", "charlie", "echo", "c"),
        ]);
        let mut config = SearchConfig::default();
        let legacy = rank_with_config(&index, "!docker", &config);
        assert_eq!(legacy.len(), 3);
        assert_eq!(legacy[0].snippet.id().as_str(), "a.md#a");
        assert!(legacy.iter().all(|hit| hit.fuzzy == Some(0)));

        config.cross_field_matching = true;
        let hits = rank_with_config(&index, "!docker", &config);
        assert_eq!(
            hits.iter()
                .map(|hit| hit.snippet.name())
                .collect::<Vec<_>>(),
            vec!["bravo", "charlie"]
        );
        assert!(
            hits.iter()
                .all(|hit| hit.fuzzy == Some(0) && hit.combined == 0.0)
        );
        let mut store = FrecencyStore::new();
        store.record(SnippetId::new("c.md", "c"), PathBuf::from("/tmp"), 1000);
        let recent = rank(&index, "!docker", &store, Path::new("/tmp"), 1000, &config);
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].snippet.name(), "charlie");
        assert_eq!(recent[0].fuzzy, Some(0));
        assert_eq!(
            recent[0].combined,
            recent[0].frecency * config.frecency_weight
        );
        assert!(recent[0].combined > recent[1].combined);
    }

    #[test]
    fn multiple_exclusions_check_every_field_even_with_zero_weights() {
        let mut files = vec![make_file("clean.md", "kitchen", "eza", "clean")];
        for field in [
            "name",
            "body",
            "description",
            "path",
            "file-name",
            "file-description",
            "tag",
        ] {
            let mut file = make_file(&format!("{field}.md"), "kitchen", "eza", "excluded");
            match field {
                "name" => file.snippets[0].name = "kitchen docker".to_string(),
                "body" => file.snippets[0].body = "eza podman".to_string(),
                "description" => file.snippets[0].description = "docker".to_string(),
                "path" => file = make_file("podman.md", "kitchen", "eza", "excluded"),
                "file-name" => file.frontmatter.name = Some("docker".to_string()),
                "file-description" => file.frontmatter.description = Some("podman".to_string()),
                "tag" => file.frontmatter.tags = vec!["clean".to_string(), "docker".to_string()],
                _ => unreachable!(),
            }
            files.push(file);
        }
        let index = SnippetIndex::from_files(files);
        let config = SearchConfig {
            cross_field_matching: true,
            fuzzy: crate::config::FuzzyWeights {
                name: 0,
                command: 0,
                description: 0,
                path: 0,
                frontmatter_name: 0,
                tag: 0,
            },
            ..SearchConfig::default()
        };
        assert_eq!(rank_with_config(&index, "kitchen eza", &config).len(), 8);
        for query in [
            "!docker !podman",
            "kitchen eza !docker !podman",
            "!podman !docker eza kitchen",
        ] {
            let hits = rank_with_config(&index, query, &config);
            assert_eq!(hits.len(), 1, "{query}");
            assert_eq!(hits[0].snippet.id().as_str(), "clean.md#clean");
            assert_eq!(hits[0].fuzzy, Some(0));
        }
    }

    #[test]
    fn cross_field_mode_preserves_scoped_operator_scores_and_restrictions() {
        let index = operator_index();
        for (query, expected) in [
            ("tag:docker logs", vec!["ops/docker.md#ship-service"]),
            ("tag:web logs", vec![]),
            ("name:ship path:ops", vec!["ops/docker.md#ship-service"]),
            ("body:docker", vec!["plain/docker-body.md#body-mention"]),
            ("name:!docker logs", vec!["ops/docker.md#ship-service"]),
            // Tag operators retain legacy best-tag semantics, not global exclusion.
            ("tag:!docker logs", vec!["ops/docker.md#ship-service"]),
            ("name:!ship logs", vec![]),
            ("tag:'docker compose'", vec![]),
            (
                "snippet:\"search google\"",
                vec!["guides/search.md#search-google"],
            ),
        ] {
            let mut config = SearchConfig::default();
            let legacy = rank_with_config(&index, query, &config);
            assert_eq!(
                legacy
                    .iter()
                    .map(|hit| hit.snippet.id().as_str())
                    .collect::<Vec<_>>(),
                expected,
                "{query}"
            );
            config.cross_field_matching = true;
            let enabled = rank_with_config(&index, query, &config);
            let scores = |hits: &[SearchHit<'_>]| {
                hits.iter()
                    .map(|hit| {
                        (
                            hit.snippet.id().as_str().to_string(),
                            hit.fuzzy,
                            hit.combined,
                        )
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(scores(&enabled), scores(&legacy), "{query}");
        }
    }

    #[test]
    fn escaped_space_atoms_stay_within_one_field_through_rank() {
        let index = SnippetIndex::from_files([
            make_file("split.md", "kitchen", "eza", "split"),
            make_file("phrase.md", "kitchen eza", "echo", "phrase"),
            make_tagged_file("tags.md", "tools", "echo", "tags", &["kitchen", "eza"]),
        ]);
        for enabled in [false, true] {
            let config = SearchConfig {
                cross_field_matching: enabled,
                ..SearchConfig::default()
            };
            for query in [r"kitchen\ eza", r"'kitchen\ eza"] {
                let hits = rank_with_config(&index, query, &config);
                assert_eq!(hits.len(), 1, "{query}, enabled={enabled}");
                assert_eq!(hits[0].snippet.id().as_str(), "phrase.md#phrase");
            }
            assert_eq!(
                rank_with_config(&index, "kitchen eza", &config).len(),
                if enabled { 3 } else { 1 }
            );
        }
    }

    #[test]
    fn cross_field_matching_preserves_anchors_exact_modifiers_and_normalization() {
        let index = SnippetIndex::from_files([make_file("a.md", "CAFÉ kitchen", "eza", "a")]);
        let config = SearchConfig {
            cross_field_matching: true,
            ..SearchConfig::default()
        };
        for query in ["^cafe eza$", "'cafe 'EZA", "kitchen$ ^eza$"] {
            assert_eq!(rank_with_config(&index, query, &config).len(), 1, "{query}");
        }
        for query in [
            "cafe$ eza",
            "^kitchen eza",
            "'cfe eza",
            "cafe ^ez$",
            "cafe !^eza$",
        ] {
            assert!(
                rank_with_config(&index, query, &config).is_empty(),
                "{query}"
            );
        }
    }

    #[test]
    fn empty_queries_and_nonempty_zero_atom_queries_preserve_score_distinction() {
        let index = tiny_index();
        for enabled in [false, true] {
            let config = SearchConfig {
                cross_field_matching: enabled,
                ..SearchConfig::default()
            };
            for query in ["", " \t ", "'", "!", "^", "$"] {
                assert!(build_pattern(query.trim()).atoms.is_empty());
                let hits = rank_with_config(&index, query, &config);
                assert_eq!(hits.len(), 3, "{query:?}, enabled={enabled}");
                let expected = if query.trim().is_empty() {
                    None
                } else {
                    Some(0)
                };
                assert!(
                    hits.iter()
                        .all(|hit| hit.fuzzy == expected && hit.combined == 0.0)
                );
            }
        }
    }

    #[test]
    fn parses_plain_text_and_empty_queries() {
        assert_eq!(
            ParsedQuery::parse("git log"),
            ParsedQuery {
                free_text: "git log".to_string(),
                terms: vec![],
            }
        );
        assert!(ParsedQuery::parse("   ").is_empty());
    }

    #[test]
    fn parses_one_operator_and_mixed_free_text() {
        assert_eq!(
            ParsedQuery::parse("tag:docker logs"),
            ParsedQuery {
                free_text: "logs".to_string(),
                terms: vec![FieldTerm {
                    field: QueryField::Tag,
                    value: "docker".to_string(),
                }],
            }
        );
    }

    #[test]
    fn parses_quoted_multi_word_operator_values() {
        assert_eq!(
            ParsedQuery::parse(
                "snippet:\"search google\" body:'legacy alias' name:'deploy service'"
            ),
            ParsedQuery {
                free_text: String::new(),
                terms: vec![
                    FieldTerm {
                        field: QueryField::Body,
                        value: "search google".to_string(),
                    },
                    FieldTerm {
                        field: QueryField::Body,
                        value: "legacy alias".to_string(),
                    },
                    FieldTerm {
                        field: QueryField::Name,
                        value: "deploy service".to_string(),
                    },
                ],
            }
        );
    }

    #[test]
    fn parses_repeated_operators() {
        let parsed = ParsedQuery::parse("tag:docker tag:compose");
        assert_eq!(parsed.free_text, "");
        assert_eq!(parsed.terms.len(), 2);
        assert_eq!(parsed.terms[0].field, QueryField::Tag);
        assert_eq!(parsed.terms[1].field, QueryField::Tag);
    }

    #[test]
    fn preserves_unknown_prefixes_and_uppercase_operators_as_free_text() {
        assert_eq!(
            ParsedQuery::parse("owner:calam TAG:docker Tag:compose").free_text,
            "owner:calam TAG:docker Tag:compose"
        );
    }

    #[test]
    fn preserves_nucleo_syntax_in_free_text_and_operator_values() {
        assert_eq!(
            ParsedQuery::parse("'apply name:bob"),
            ParsedQuery {
                free_text: "'apply".to_string(),
                terms: vec![FieldTerm {
                    field: QueryField::Name,
                    value: "bob".to_string(),
                }],
            }
        );
        assert_eq!(
            ParsedQuery::parse("name:'bob").terms[0].value,
            "'bob".to_string()
        );
    }

    #[test]
    fn treats_empty_operator_values_as_free_text() {
        assert_eq!(ParsedQuery::parse("tag:").free_text, "tag:");
    }

    #[test]
    fn highlight_terms_follow_query_syntax() {
        assert_eq!(
            highlight_terms("name:prompt"),
            vec![HighlightTerm {
                field: Some(QueryField::Name),
                value: "prompt".to_string(),
            }]
        );
        assert_eq!(
            highlight_terms("snippet:\"search google\""),
            vec![HighlightTerm {
                field: Some(QueryField::Body),
                value: "search google".to_string(),
            }]
        );
        assert_eq!(
            highlight_terms("tag:docker logs"),
            vec![
                HighlightTerm {
                    field: None,
                    value: "logs".to_string(),
                },
                HighlightTerm {
                    field: Some(QueryField::Tag),
                    value: "docker".to_string(),
                },
            ]
        );
        assert!(highlight_terms("   ").is_empty());
    }

    #[test]
    fn tag_operator_excludes_body_only_mentions() {
        let ids = ranked_ids(&operator_index(), "tag:docker");
        assert!(ids.contains(&"ops/docker.md#ship-service".to_string()));
        assert!(!ids.contains(&"plain/docker-body.md#body-mention".to_string()));
    }

    #[test]
    fn name_operator_excludes_body_and_path_only_matches() {
        let ids = ranked_ids(&operator_index(), "name:foo");
        assert_eq!(ids, vec!["name-only.md#foo-command"]);
    }

    #[test]
    fn path_operator_excludes_name_and_body_only_matches() {
        let ids = ranked_ids(&operator_index(), "path:foo");
        assert_eq!(ids, vec!["foo/bar.md#path-only"]);
    }

    #[test]
    fn body_operator_excludes_name_and_path_only_matches() {
        let ids = ranked_ids(&operator_index(), "body:docker");
        assert_eq!(ids, vec!["plain/docker-body.md#body-mention"]);
    }

    #[test]
    fn snippet_operator_matches_body_code() {
        let ids = ranked_ids(&operator_index(), "snippet:docker");
        assert_eq!(ids, vec!["plain/docker-body.md#body-mention"]);
    }

    #[test]
    fn mixed_operator_and_free_text_requires_both() {
        let ids = ranked_ids(&operator_index(), "tag:docker logs");
        assert_eq!(ids, vec!["ops/docker.md#ship-service"]);
        assert!(ranked_ids(&operator_index(), "tag:web logs").is_empty());
    }

    #[test]
    fn quoted_field_values_match_only_that_field() {
        let ids = ranked_ids(&operator_index(), "snippet:\"search google\"");
        assert_eq!(ids, vec!["guides/search.md#search-google"]);
    }

    #[test]
    fn nucleo_syntax_is_retained_in_ranked_queries() {
        let index = SnippetIndex::from_files([
            make_file("a.md", "bob deploy", "apply", "bob-deploy"),
            make_file("b.md", "alice deploy", "supply", "alice-deploy"),
            make_file("c.md", "charlie deploy", "apply", "charlie-deploy"),
        ]);
        assert_eq!(
            ranked_ids(&index, "'apply name:bob"),
            vec!["a.md#bob-deploy"]
        );
        assert_eq!(ranked_ids(&index, "name:'bob"), vec!["a.md#bob-deploy"]);
    }

    #[test]
    fn multiple_operator_terms_are_anded() {
        let index = operator_index();
        assert_eq!(
            ranked_ids(&index, "tag:docker tag:compose"),
            vec!["ops/docker.md#ship-service"]
        );
        assert_eq!(
            ranked_ids(&index, "tag:docker body:logs"),
            vec!["ops/docker.md#ship-service"]
        );
        assert_eq!(
            ranked_ids(&index, "name:ship path:ops"),
            vec!["ops/docker.md#ship-service"]
        );
        assert!(ranked_ids(&index, "tag:docker tag:missing").is_empty());
    }

    #[test]
    fn unknown_prefix_matches_through_full_text_only() {
        let ids = ranked_ids(&operator_index(), "owner:calam");
        assert_eq!(ids, vec!["owners/calam.md#ownership-note"]);
    }

    #[test]
    fn operator_scores_feed_ranking() {
        let index = SnippetIndex::from_files([
            make_tagged_file(
                "docker/a.md",
                "deploy docker",
                "docker logs",
                "a",
                &["docker"],
            ),
            make_tagged_file(
                "docker/b.md",
                "deploy",
                "docker logs compose",
                "b",
                &["docker", "compose"],
            ),
        ]);

        let plain = rank(
            &index,
            "docker",
            &FrecencyStore::new(),
            Path::new("/tmp"),
            0,
            &SearchConfig::default(),
        );
        let single = rank(
            &index,
            "tag:docker",
            &FrecencyStore::new(),
            Path::new("/tmp"),
            0,
            &SearchConfig::default(),
        );
        let mixed = rank(
            &index,
            "tag:docker logs",
            &FrecencyStore::new(),
            Path::new("/tmp"),
            0,
            &SearchConfig::default(),
        );
        let repeated = rank(
            &index,
            "tag:docker tag:compose",
            &FrecencyStore::new(),
            Path::new("/tmp"),
            0,
            &SearchConfig::default(),
        );

        assert!(plain[0].fuzzy.unwrap() > 0);
        assert!(single[0].fuzzy.unwrap() > 0);
        assert!(mixed[0].fuzzy.unwrap() > single[0].fuzzy.unwrap());
        assert_eq!(repeated.len(), 1);
        assert_eq!(repeated[0].snippet.id().as_str(), "docker/b.md#b");
        assert!(repeated[0].fuzzy.unwrap() > single[0].fuzzy.unwrap());
    }
}
