use crate::config::SearchConfig;
use crate::frecency::FrecencyStore;
use crate::fuzzy::{FuzzyScorer, build_pattern, score_snippet};
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

/// Rank every snippet in `index` for the given `query` and `cwd`.
///
/// 1. Each snippet is scored with [`score_snippet`] (fuzzy match over
///    weighted fields). Non-matching snippets are dropped entirely.
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
    let pattern = build_pattern(query);
    let empty = query.is_empty();

    let mut hits: Vec<SearchHit<'a>> = index
        .iter()
        .filter_map(|entry| {
            let fuzzy = score_snippet(&mut scorer, &pattern, empty, entry, &config.fuzzy)?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SearchConfig;
    use crate::domain::{Frontmatter, Snippet, SnippetFile, SnippetId};
    use crate::index::SnippetIndex;
    use std::path::PathBuf;

    fn make_file(rel: &str, heading: &str, body: &str, slug: &str) -> SnippetFile {
        SnippetFile {
            path: PathBuf::from(rel),
            relative_path: PathBuf::from(rel),
            frontmatter: Frontmatter::default(),
            snippets: vec![Snippet {
                id: SnippetId::new(rel, slug),
                name: heading.to_string(),
                description: String::new(),
                body: body.to_string(),
                variables: vec![],
            }],
        }
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
}
