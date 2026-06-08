# Query Syntax

The fuzzy search bar accepts two complementary syntaxes that can be freely combined: **field operators** (specific to peanutbutter) and **nucleo pattern modifiers** (passed through to the underlying matcher).

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

**Operators combine with free text**, which is matched across all fields:

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
| `!word` | Inverse fuzzy — exclude entries that match `word` |
| `!^word` | Inverse prefix |
| `!word$` | Inverse suffix |

Modifiers work inside field operators too:

```
name:'apply                     # name contains exact substring "apply"
name:^git                       # name starts with "git"
command:!docker                 # command block does not fuzzy-match "docker"
```

---

## Scoring and Ranking

When the query is empty, results are ordered purely by frecency (recency × frequency × location affinity).

When the query is non-empty:

- Each field is scored independently and **weighted** — name matches rank higher than command-block matches by default.
- Field operator scores are added to the free-text score, so a query that matches via both free text and an explicit operator ranks higher.
- The final score is `fuzzy_score + frecency_score × frecency_weight`.

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
