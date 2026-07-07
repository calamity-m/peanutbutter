//! Minimal glob matching shared by lint suppression and discovery ignores.

/// Match `value` against `pattern`, where `*` matches any run of characters
/// (including none) and `?` matches exactly one. Matching is byte-wise and
/// case-sensitive; there is no special handling of `/`.
pub(crate) fn glob_matches(pattern: &str, value: &str) -> bool {
    glob_matches_bytes(pattern.as_bytes(), value.as_bytes())
}

fn glob_matches_bytes(pattern: &[u8], value: &[u8]) -> bool {
    match (pattern, value) {
        ([], []) => true,
        ([], _) => false,
        ([b'*', rest @ ..], _) => {
            glob_matches_bytes(rest, value)
                || (!value.is_empty() && glob_matches_bytes(pattern, &value[1..]))
        }
        ([b'?', rest @ ..], [_, value_rest @ ..]) => glob_matches_bytes(rest, value_rest),
        ([p, rest @ ..], [v, value_rest @ ..]) if p == v => glob_matches_bytes(rest, value_rest),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_literals_stars_and_question_marks() {
        assert!(glob_matches("a/b.md", "a/b.md"));
        assert!(glob_matches("*", "anything"));
        assert!(glob_matches("a/*", "a/deep/nested"));
        assert!(glob_matches("?.md", "a.md"));
        assert!(!glob_matches("a/b", "a/bc"));
        assert!(!glob_matches("?.md", "ab.md"));
    }
}
