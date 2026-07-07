use crate::config::Paths;
use crate::domain::SnippetId;
use crate::frecency::FrecencyStore;
use crate::index::SnippetIndex;
use std::collections::{BTreeMap, HashSet};
use std::io::{self, BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Runtime options for frecency garbage collection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcOptions {
    /// Report changes without modifying the frecency store.
    pub dry_run: bool,
    /// Remove unresolved orphaned events without an extra purge prompt.
    pub purge: bool,
    /// Use compact output suitable for scripts.
    pub quiet: bool,
}

/// One orphaned frecency id discovered without mutating the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcOrphan {
    /// Orphaned snippet id from the frecency store.
    pub id: SnippetId,
    /// Number of events using this orphaned id.
    pub events: usize,
    /// Best current snippet id that GC would offer for reattachment, if any.
    pub candidate_id: Option<SnippetId>,
}

/// Summary of a garbage-collection run over the frecency store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GcResult {
    /// Number of orphaned events found before any changes.
    pub orphan_events: usize,
    /// Number of distinct orphaned snippet ids found.
    pub orphan_ids: usize,
    /// Number of events reattached to a current snippet id.
    pub reattached_events: usize,
    /// Number of events removed from the store.
    pub purged_events: usize,
    /// Backup file written before saving changes, if any.
    pub backup_path: Option<PathBuf>,
    /// Whether the frecency store was saved.
    pub saved: bool,
}

/// Run garbage collection for orphaned frecency events using stdin/stdout for
/// interactive reattachment prompts.
pub fn run<W: Write>(paths: &Paths, options: GcOptions, writer: &mut W) -> io::Result<GcResult> {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    run_with(paths, options, &mut input, writer)
}

/// Testable variant of [`run`] that accepts explicit input/output.
pub fn run_with<R: BufRead, W: Write>(
    paths: &Paths,
    options: GcOptions,
    input: &mut R,
    writer: &mut W,
) -> io::Result<GcResult> {
    let index = crate::index::load_from_paths(paths)?;
    let mut store = FrecencyStore::load(&paths.state_file)?;
    let orphan_counts = orphan_counts(&index, &store);

    let orphan_events = orphan_counts.values().sum();
    print_header(
        paths,
        &index,
        orphan_events,
        orphan_counts.len(),
        options.quiet,
        writer,
    )?;

    let mut reattached_events = 0;
    let mut unresolved = Vec::new();
    for (old_id, count) in &orphan_counts {
        let candidate = best_candidate(old_id, &index);
        let Some(candidate) = candidate else {
            print_unresolved(old_id, *count, options.quiet, writer)?;
            unresolved.push(old_id.clone());
            continue;
        };

        if options.dry_run {
            print_dry_run_candidate(old_id, candidate.id(), *count, options.quiet, writer)?;
            unresolved.push(old_id.clone());
            continue;
        }

        prompt_reattach(old_id, candidate.id(), *count, options.quiet, writer)?;
        let mut answer = String::new();
        input.read_line(&mut answer)?;
        if is_yes(&answer) {
            reattached_events += store.reattach(old_id, candidate.id());
            writeln!(writer, "reattached")?;
        } else {
            writeln!(writer, "skipped")?;
            unresolved.push(old_id.clone());
        }
    }

    let purged_events = handle_purge(
        &mut store,
        &orphan_counts,
        &unresolved,
        options,
        input,
        writer,
    )?;

    let would_change = reattached_events > 0 || (!options.dry_run && purged_events > 0);
    let mut backup_path = None;
    let mut saved = false;
    if would_change {
        backup_path = FrecencyStore::backup(&paths.state_file, unix_now())?;
        if options.quiet
            && let Some(path) = &backup_path
        {
            writeln!(writer, "backed up frecency store to {}", path.display())?;
        }
        store.save(&paths.state_file)?;
        saved = true;
        if options.quiet {
            writeln!(writer, "saved frecency store")?;
        }
    }

    print_result(
        GcResultView {
            reattached_events,
            purged_events,
            backup_path: backup_path.as_deref(),
            dry_run: options.dry_run,
            saved,
            would_change,
            quiet: options.quiet,
        },
        writer,
    )?;

    Ok(GcResult {
        orphan_events,
        orphan_ids: orphan_counts.len(),
        reattached_events,
        purged_events,
        backup_path,
        saved,
    })
}

/// Collect orphaned frecency ids and likely reattachment candidates without
/// prompting, purging, saving, or writing formatted output.
pub fn collect_orphans(paths: &Paths) -> io::Result<Vec<GcOrphan>> {
    let index = crate::index::load_from_paths(paths)?;
    collect_orphans_with_index(paths, &index)
}

/// Collect orphaned frecency ids using an already-loaded snippet index.
pub fn collect_orphans_with_index(
    paths: &Paths,
    index: &SnippetIndex,
) -> io::Result<Vec<GcOrphan>> {
    let store = FrecencyStore::load(&paths.state_file)?;
    let orphan_counts = orphan_counts(index, &store);
    Ok(orphan_counts
        .into_iter()
        .map(|(id, events)| GcOrphan {
            candidate_id: best_candidate(&id, index).map(|candidate| candidate.id().clone()),
            id,
            events,
        })
        .collect())
}

fn orphan_counts(index: &SnippetIndex, store: &FrecencyStore) -> BTreeMap<SnippetId, usize> {
    let known_ids: HashSet<_> = index.iter().map(|entry| entry.id().clone()).collect();
    let mut orphan_counts: BTreeMap<SnippetId, usize> = BTreeMap::new();
    for event in store.events() {
        if !known_ids.contains(&event.id) {
            *orphan_counts.entry(event.id.clone()).or_default() += 1;
        }
    }
    orphan_counts
}

fn handle_purge<R: BufRead, W: Write>(
    store: &mut FrecencyStore,
    orphan_counts: &BTreeMap<SnippetId, usize>,
    unresolved: &[SnippetId],
    options: GcOptions,
    input: &mut R,
    writer: &mut W,
) -> io::Result<usize> {
    let unresolved_events: usize = unresolved
        .iter()
        .filter_map(|id| orphan_counts.get(id))
        .sum();
    if options.purge {
        if options.dry_run {
            print_dry_run_purge(unresolved_events, options.quiet, writer)?;
            return Ok(unresolved_events);
        }
        let purged_events = store.purge_ids(unresolved);
        if purged_events > 0 && options.quiet {
            writeln!(
                writer,
                "purged {purged_events} unresolved orphaned event(s)"
            )?;
        }
        return Ok(purged_events);
    }

    if options.dry_run || unresolved_events == 0 {
        return Ok(0);
    }

    prompt_purge(unresolved_events, options.quiet, writer)?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    if is_yes(&answer) {
        let purged_events = store.purge_ids(unresolved);
        writeln!(
            writer,
            "purged {purged_events} unresolved orphaned event(s)"
        )?;
        Ok(purged_events)
    } else {
        writeln!(writer, "kept unresolved orphaned event(s)")?;
        Ok(0)
    }
}

fn print_header<W: Write>(
    paths: &Paths,
    index: &SnippetIndex,
    orphan_events: usize,
    orphan_ids: usize,
    quiet: bool,
    writer: &mut W,
) -> io::Result<()> {
    if quiet {
        writeln!(
            writer,
            "found {orphan_events} orphaned frecency event(s) across {orphan_ids} snippet id(s)"
        )
    } else {
        writeln!(writer, "Peanutbutter frecency GC")?;
        writeln!(writer)?;
        writeln!(writer, "State:    {}", paths.state_file.display())?;
        writeln!(writer, "Snippets: {} known", index.len())?;
        writeln!(
            writer,
            "Orphans:  {orphan_events} event(s) across {orphan_ids} snippet id(s)"
        )
    }
}

fn print_unresolved<W: Write>(
    old_id: &SnippetId,
    count: usize,
    quiet: bool,
    writer: &mut W,
) -> io::Result<()> {
    if quiet {
        writeln!(writer, "unresolved: {old_id} ({count} event(s))")
    } else {
        writeln!(writer)?;
        writeln!(writer, "Unresolved")?;
        writeln!(writer, "  {old_id}")?;
        writeln!(writer, "  events: {count}")
    }
}

fn print_dry_run_candidate<W: Write>(
    old_id: &SnippetId,
    new_id: &SnippetId,
    count: usize,
    quiet: bool,
    writer: &mut W,
) -> io::Result<()> {
    if quiet {
        writeln!(
            writer,
            "would prompt to reattach {old_id} -> {new_id} ({count} event(s))"
        )
    } else {
        writeln!(writer)?;
        writeln!(writer, "Candidate")?;
        writeln!(writer, "  from:   {old_id}")?;
        writeln!(writer, "  to:     {new_id}")?;
        writeln!(writer, "  events: {count}")?;
        writeln!(writer, "  action: would prompt for reattachment")
    }
}

fn prompt_reattach<W: Write>(
    old_id: &SnippetId,
    new_id: &SnippetId,
    count: usize,
    quiet: bool,
    writer: &mut W,
) -> io::Result<()> {
    if quiet {
        write!(
            writer,
            "reattach {old_id} -> {new_id} ({count} event(s))? [y/N] "
        )?;
    } else {
        writeln!(writer)?;
        writeln!(writer, "Candidate")?;
        writeln!(writer, "  from:   {old_id}")?;
        writeln!(writer, "  to:     {new_id}")?;
        writeln!(writer, "  events: {count}")?;
        write!(writer, "  Reattach? [y/N] ")?;
    }
    writer.flush()
}

fn print_dry_run_purge<W: Write>(
    unresolved_events: usize,
    quiet: bool,
    writer: &mut W,
) -> io::Result<()> {
    if quiet {
        writeln!(
            writer,
            "would purge {unresolved_events} unresolved orphaned event(s)"
        )
    } else {
        writeln!(writer)?;
        writeln!(writer, "Cleanup")?;
        writeln!(
            writer,
            "  action: would purge {unresolved_events} unresolved orphaned event(s)"
        )
    }
}

fn prompt_purge<W: Write>(unresolved_events: usize, quiet: bool, writer: &mut W) -> io::Result<()> {
    if quiet {
        write!(
            writer,
            "purge {unresolved_events} unresolved orphaned event(s)? [y/N] "
        )?;
    } else {
        writeln!(writer)?;
        writeln!(writer, "Cleanup")?;
        write!(
            writer,
            "  Purge {unresolved_events} unresolved orphaned event(s)? [y/N] "
        )?;
    }
    writer.flush()
}

struct GcResultView<'a> {
    reattached_events: usize,
    purged_events: usize,
    backup_path: Option<&'a Path>,
    dry_run: bool,
    saved: bool,
    would_change: bool,
    quiet: bool,
}

fn print_result<W: Write>(view: GcResultView<'_>, writer: &mut W) -> io::Result<()> {
    if view.quiet {
        if !view.would_change {
            if view.dry_run {
                writeln!(writer, "dry run: no changes written")?;
            } else {
                writeln!(writer, "no changes written")?;
            }
        }
        return Ok(());
    }

    writeln!(writer)?;
    writeln!(writer, "Result")?;
    writeln!(writer, "  Reattached: {} event(s)", view.reattached_events)?;
    writeln!(writer, "  Purged:     {} event(s)", view.purged_events)?;
    if let Some(path) = view.backup_path {
        writeln!(writer, "  Backup:     {}", path.display())?;
    }
    if view.dry_run {
        writeln!(writer, "  Saved:      no (dry run)")?;
    } else if view.saved {
        writeln!(writer, "  Saved:      yes")?;
    } else {
        writeln!(writer, "  Saved:      no changes")?;
    }
    Ok(())
}

fn best_candidate<'a>(
    old_id: &SnippetId,
    index: &'a SnippetIndex,
) -> Option<&'a crate::index::IndexedSnippet> {
    let old_slug = id_slug(old_id);
    let old_path = id_path(old_id);
    let old_file = Path::new(old_path)
        .file_name()
        .and_then(|name| name.to_str());
    index
        .iter()
        .filter_map(|entry| {
            let new_slug = id_slug(entry.id());
            let distance = edit_distance(old_slug, new_slug);
            let max_len = old_slug.len().max(new_slug.len());
            // Treat small edits as likely renames, but avoid suggesting wildly
            // different snippets just because they are the nearest available id.
            let threshold = 2usize.max(max_len / 3);
            if distance > threshold {
                return None;
            }
            let same_file = old_file
                .zip(
                    entry
                        .relative_path
                        .file_name()
                        .and_then(|name| name.to_str()),
                )
                .is_some_and(|(old, new)| old == new);
            Some((entry, distance, !same_file))
        })
        .min_by_key(|(_, distance, different_file)| (*distance, *different_file))
        .map(|(entry, _, _)| entry)
}

fn id_slug(id: &SnippetId) -> &str {
    id.as_str()
        .split_once('#')
        .map(|(_, slug)| slug)
        .unwrap_or("")
}

fn id_path(id: &SnippetId) -> &str {
    id.as_str()
        .split_once('#')
        .map(|(path, _)| path)
        .unwrap_or("")
}

// Returns the minimum number of single-character insertions, deletions, or
// substitutions needed to turn one string into the other.
fn edit_distance(a: &str, b: &str) -> usize {
    let b_chars: Vec<char> = b.chars().collect();
    let mut costs: Vec<usize> = (0..=b_chars.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = costs[0];
        costs[0] = i + 1;
        for (j, &cb) in b_chars.iter().enumerate() {
            let insertion = costs[j + 1] + 1;
            let deletion = costs[j] + 1;
            let substitution = previous + usize::from(ca != cb);
            previous = costs[j + 1];
            costs[j + 1] = insertion.min(deletion).min(substitution);
        }
    }
    costs[b_chars.len()]
}

fn is_yes(answer: &str) -> bool {
    answer.trim().eq_ignore_ascii_case("y") || answer.trim().eq_ignore_ascii_case("yes")
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Paths;
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn temp_dir(prefix: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let path = std::env::temp_dir().join(format!(
            "pb-{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn test_paths(root: &Path) -> Paths {
        Paths {
            snippet_roots: vec![root.to_path_buf()],
            xdg_snippets_dir: root.to_path_buf(),
            snippet_overrides_active: false,
            ignored: Vec::new(),
            state_file: root.join("state.tsv"),
            config_file: root.join("config.toml"),
        }
    }

    fn opts(dry_run: bool, purge: bool, quiet: bool) -> GcOptions {
        GcOptions {
            dry_run,
            purge,
            quiet,
        }
    }

    #[test]
    fn dry_run_reports_orphans_without_writing() {
        let root = temp_dir("gc-dry-run");
        fs::write(root.join("new.md"), "## Echo\n\n```\necho hi\n```\n").unwrap();
        let paths = test_paths(&root);
        let mut store = FrecencyStore::new();
        store.record(SnippetId::new("old.md", "echo"), PathBuf::from("/repo"), 1);
        store.save(&paths.state_file).unwrap();
        let before = fs::read_to_string(&paths.state_file).unwrap();

        let mut input = io::Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let result = run_with(&paths, opts(true, true, true), &mut input, &mut output).unwrap();

        assert_eq!(result.orphan_events, 1);
        assert_eq!(result.orphan_ids, 1);
        assert_eq!(result.reattached_events, 0);
        assert_eq!(result.purged_events, 1);
        assert!(!result.saved);
        assert!(result.backup_path.is_none());
        assert_eq!(fs::read_to_string(&paths.state_file).unwrap(), before);
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("would prompt to reattach old.md#echo -> new.md#echo"));
        assert!(output.contains("dry run: no changes written"));
    }

    #[test]
    fn reattaches_confirmed_orphan_and_backs_up_store() {
        let root = temp_dir("gc-reattach");
        fs::write(root.join("new.md"), "## Echo\n\n```\necho hi\n```\n").unwrap();
        let paths = test_paths(&root);
        let mut store = FrecencyStore::new();
        store.record(SnippetId::new("old.md", "echo"), PathBuf::from("/repo"), 1);
        store.save(&paths.state_file).unwrap();
        let before = fs::read_to_string(&paths.state_file).unwrap();

        let mut input = io::Cursor::new(b"y\n".to_vec());
        let mut output = Vec::new();
        let result = run_with(&paths, opts(false, false, false), &mut input, &mut output).unwrap();

        assert_eq!(result.reattached_events, 1);
        assert_eq!(result.purged_events, 0);
        assert!(result.saved);
        let backup_path = result.backup_path.unwrap();
        assert_eq!(fs::read_to_string(backup_path).unwrap(), before);
        let saved = FrecencyStore::load(&paths.state_file).unwrap();
        assert_eq!(saved.events()[0].id.as_str(), "new.md#echo");
    }

    #[test]
    fn prompts_to_purge_unresolved_orphans() {
        let root = temp_dir("gc-purge-prompt");
        fs::write(root.join("new.md"), "## Echo\n\n```\necho hi\n```\n").unwrap();
        let paths = test_paths(&root);
        let mut store = FrecencyStore::new();
        store.record(
            SnippetId::new("missing.md", "completely-different"),
            PathBuf::from("/repo"),
            1,
        );
        store.save(&paths.state_file).unwrap();

        let mut input = io::Cursor::new(b"y\n".to_vec());
        let mut output = Vec::new();
        let result = run_with(&paths, opts(false, false, false), &mut input, &mut output).unwrap();

        assert_eq!(result.reattached_events, 0);
        assert_eq!(result.purged_events, 1);
        assert!(result.saved);
        assert!(
            FrecencyStore::load(&paths.state_file)
                .unwrap()
                .events()
                .is_empty()
        );
        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Cleanup"));
        assert!(output.contains("Purge 1 unresolved orphaned event(s)? [y/N]"));
        assert!(output.contains("Result"));
    }

    #[test]
    fn purges_unresolved_orphans_when_requested() {
        let root = temp_dir("gc-purge");
        fs::write(root.join("new.md"), "## Echo\n\n```\necho hi\n```\n").unwrap();
        let paths = test_paths(&root);
        let mut store = FrecencyStore::new();
        store.record(
            SnippetId::new("missing.md", "completely-different"),
            PathBuf::from("/repo"),
            1,
        );
        store.save(&paths.state_file).unwrap();

        let mut input = io::Cursor::new(Vec::<u8>::new());
        let mut output = Vec::new();
        let result = run_with(&paths, opts(false, true, false), &mut input, &mut output).unwrap();

        assert_eq!(result.reattached_events, 0);
        assert_eq!(result.purged_events, 1);
        assert!(result.saved);
        assert!(
            FrecencyStore::load(&paths.state_file)
                .unwrap()
                .events()
                .is_empty()
        );
    }
}
