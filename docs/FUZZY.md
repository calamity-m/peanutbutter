# Query Syntax

The fuzzy search bar accepts two complementary syntaxes that can be freely combined: **field operators** (specific to peanutbutter) and **nucleo pattern modifiers** (passed through to the underlying matcher).

---

## Free-Text Matching

Free text searches the snippet name, command block, description, relative path, optional frontmatter name/description, and each individual tag. Language and heading slug are not free-text fields.

By default, the **whole free-text pattern must match within at least one field**. Terms cannot be satisfied by different fields, and exclusions apply within each field's pattern match. Omitting `cross_field_matching` or setting it to `false` preserves this behaviour, scores, ordering, and highlighting.

Opt in through your TOML config:

```toml
[search]
cross_field_matching = true
```

This switch is **TOML-only**; it is not available in `pb settings` or as a CLI flag.

With it enabled:

- Positive terms (Nucleo atoms) are **AND-ed across fields**: every atom must match at least one field, but different atoms may match different fields or tags. A single atom, including one containing an escaped space, must still match within one field; fields and tags are not concatenated.
- An **unscoped negative atom applies snippet-wide**: a match in any searchable field excludes the snippet. All negative atoms must pass. Plain `!word` excludes a substring, not a fuzzy subsequence.
- A negative-only query returns surviving snippets with fuzzy score zero, ordered by frecency and the usual tie-breaking.
- Each positive atom scores only its best weighted field. This changes ranking as well as matching, even for single-term queries; see [Scoring and Ranking](#scoring-and-ranking).

For example, `kitchen eza` finds a snippet named `kitchen sink` in `files/eza.md` even when no field contains both terms. `kitchen !docker` requires a `kitchen` match somewhere and excludes the snippet if any searchable field contains `docker`, including a tag. For an explicitly scoped alternative in **either mode**, use `name:kitchen path:eza`.

Field operators and their scoring are unchanged by this option.

---

## Field Operators

Scope a term to a specific snippet field. Unrecognised or uppercase prefixes are treated as plain free text.

| Operator | Field searched |
|----------|---------------|
| `name:term` | Snippet heading / name |
| `path:term` | Relative file path |
| `tag:term` | Frontmatter tags |
| `command:term` | Executable snippet command block |
| `snippet:term` | Executable snippet command block (alias for `command:`) |
| `body:term` | Deprecated alias for `command:` |

**Multi-word values** must be quoted with `"…"` or `'…'`:

```
name:"deploy service"
snippet:'kubectl logs'
```

**Multiple operators are AND-ed** — every term must match:

```
tag:docker tag:compose          # must have both tags
name:deploy path:ops            # heading matches "deploy" AND path contains "ops"
tag:docker command:logs         # docker tag AND "logs" in the command block
```

**Operators combine with free text**, using the free-text matching mode described above:

```
tag:docker logs                 # docker tag AND "logs" anywhere
```

---

## Nucleo Pattern Modifiers

These are passed directly to the [nucleo](https://github.com/helix-editor/nucleo) fuzzy matcher. They work in free text and inside field operator values.

| Syntax | Meaning |
|--------|---------|
| `word` | Fuzzy match — characters appear in order, gaps allowed |
| `'word` | Exact substring match |
| `^word` | Prefix match — haystack must start with `word` |
| `word$` | Suffix match — haystack must end with `word` |
| `^word$` | Exact match |
| `!word` | Inverse substring — exclude a contiguous `word` match (snippet-wide for unscoped terms in cross-field mode) |
| `!^word` | Inverse prefix |
| `!word$` | Inverse suffix |

Modifiers work inside field operators too:

```
name:'apply                     # name contains exact substring "apply"
name:^git                       # name starts with "git"
command:!docker                 # command block does not contain substring "docker"
```

---

## Scoring and Ranking

When the query is empty, results are ordered purely by frecency (recency × frequency × location affinity).

When the query is non-empty:

- In default mode, each field matching the whole free-text pattern contributes its **weighted** score — name matches rank higher than command-block matches by default.
- In cross-field mode, each positive atom contributes only its **best weighted field score** (`max(raw_score × field_weight)`), then scores are summed across atoms. Repeating an atom in additional fields or tags does not stack up extra points. Negative atoms add no score; zero-weight fields still count for matching and exclusions.
- Field operator scores are added to the free-text score, so a query that matches via both free text and an explicit operator ranks higher.
- The final score is `fuzzy_score + frecency_score × frecency_weight`.

Default field weights remain name **30**, tag **20**, frontmatter name **15**, description **10**, path **10**, and command **8**. Best-field scoring prevents repeated metadata from overwhelming a stronger name match merely by appearing in more fields. It does not guarantee exact-name-first ordering for every query, custom weight configuration, or usage history.

**Compatibility:** best-field scoring differs from default-mode accumulation even for single-term queries: `docker` scores **4980** rather than **6308** for a snippet named `docker ps` with command `docker ps -a` at `plain.md`. It also replaces the earlier cross-field implementation's all-field accumulation. Cross-field matching and exclusions are unchanged, as are default-mode and field-operator scores. Since `frecency_weight` is unchanged (default **250**), lower fuzzy scores can give usage history more relative influence; tune `[search] frecency_weight` if needed.

Matching is **case-insensitive** with Unicode smart normalisation (accented characters match their ASCII base).

---

## Examples

```
docker                          # fuzzy match across all fields
'kubectl apply                  # exact substring anywhere
name:deploy path:infra          # heading has "deploy", path has "infra"
tag:docker tag:compose logs     # both tags present, "logs" anywhere
command:"kubectl logs"          # exact phrase in the command block
name:^git command:!rebase       # name starts with "git", command excludes "rebase"
```
