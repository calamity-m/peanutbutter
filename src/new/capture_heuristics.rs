//! Variable-detection heuristics and placeholder rendering for `pb new`.
//!
//! Pure functions, no I/O. Given a captured command string, produce a list of
//! [`TokenCandidate`]s the user can confirm or reject in the capture TUI, and
//! render the final snippet body with `<@name>` placeholders substituted in.

use std::collections::HashMap;

/// A byte span `[start, end)` over the raw captured command string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

/// Classification of a detected token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenKind {
    Ipv4,
    Ipv6,
    Url,
    Path,
    QuotedString,
    Semver,
    KeyValue,
    LongHex,
    Secret,
}

/// A heuristic-detected token candidate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenCandidate {
    /// Byte span the placeholder will replace.
    pub span: Span,
    /// Substring of `raw` covered by `span`.
    pub original: String,
    /// Classification.
    pub kind: TokenKind,
    /// Heuristic-derived placeholder name with unique suffix already applied.
    pub suggested_name: String,
    /// Whether the candidate should start selected (secrets only).
    pub default_selected: bool,
}

/// Detect candidate variables in `raw`. Returns candidates in span order with
/// name collisions already resolved.
pub fn detect_variables(raw: &str) -> Vec<TokenCandidate> {
    let tokens = tokenize_with_spans(raw);
    let mut hits: Vec<RawHit> = Vec::new();

    let mut i = 0;
    while i < tokens.len() {
        let (span, ref text) = tokens[i];
        // --flag=value secret form
        if let Some((flag, eq_off)) = split_flag_eq(text)
            && is_secret_flag(flag)
        {
            let value_span = Span {
                start: span.start + eq_off,
                end: span.end,
            };
            let value = raw[value_span.start..value_span.end].to_string();
            hits.push(RawHit::new(
                value_span,
                TokenKind::Secret,
                value,
                None,
                true,
            ));
            i += 1;
            continue;
        }
        // --flag VALUE secret form
        if is_secret_flag(text)
            && let Some((next_span, next_text)) = tokens.get(i + 1).cloned()
        {
            hits.push(RawHit::new(
                next_span,
                TokenKind::Secret,
                next_text,
                None,
                true,
            ));
            i += 2;
            continue;
        }
        // Authorization: Bearer <value>
        if text.eq_ignore_ascii_case("Authorization:")
            && let Some((_, scheme)) = tokens.get(i + 1).cloned()
            && scheme.eq_ignore_ascii_case("Bearer")
            && let Some((tok_span, tok_text)) = tokens.get(i + 2).cloned()
        {
            hits.push(RawHit::new(
                tok_span,
                TokenKind::Secret,
                tok_text,
                None,
                true,
            ));
            i += 3;
            continue;
        }
        if let Some(kind) = classify_token(text) {
            match kind {
                TokenKind::KeyValue => {
                    if let Some(eq_idx) = text.find('=') {
                        let key = &text[..eq_idx];
                        let value_span = Span {
                            start: span.start + eq_idx + 1,
                            end: span.end,
                        };
                        let value = raw[value_span.start..value_span.end].to_string();
                        hits.push(RawHit::new(
                            value_span,
                            TokenKind::KeyValue,
                            value,
                            Some(slug_lower(key)),
                            false,
                        ));
                    }
                }
                TokenKind::QuotedString => {
                    if text.len() >= 2 {
                        let inner_span = Span {
                            start: span.start + 1,
                            end: span.end - 1,
                        };
                        let inner = raw[inner_span.start..inner_span.end].to_string();
                        let (k, sel) = if is_likely_base64_secret(&inner) {
                            (TokenKind::Secret, true)
                        } else {
                            (TokenKind::QuotedString, false)
                        };
                        hits.push(RawHit::new(inner_span, k, inner, None, sel));
                    }
                }
                TokenKind::LongHex if is_likely_base64_secret(text) => {
                    hits.push(RawHit::new(
                        span,
                        TokenKind::Secret,
                        text.clone(),
                        None,
                        true,
                    ));
                }
                _ => {
                    hits.push(RawHit::new(span, kind, text.clone(), None, false));
                }
            }
        } else if is_likely_base64_secret(text) {
            hits.push(RawHit::new(
                span,
                TokenKind::Secret,
                text.clone(),
                None,
                true,
            ));
        }
        i += 1;
    }

    hits.sort_by_key(|h| (h.span.start, h.span.end));
    let mut filtered: Vec<RawHit> = Vec::new();
    let mut last_end = 0;
    for h in hits {
        if h.span.start < last_end {
            continue;
        }
        last_end = h.span.end;
        filtered.push(h);
    }

    let mut names_seen: HashMap<String, usize> = HashMap::new();
    filtered
        .into_iter()
        .map(|h| {
            let base = h.override_name.unwrap_or_else(|| name_for_kind(h.kind));
            let name = bump_name(&base, &mut names_seen);
            TokenCandidate {
                span: h.span,
                original: h.original,
                kind: h.kind,
                suggested_name: name,
                default_selected: h.default_selected,
            }
        })
        .collect()
}

struct RawHit {
    span: Span,
    kind: TokenKind,
    original: String,
    override_name: Option<String>,
    default_selected: bool,
}

impl RawHit {
    fn new(
        span: Span,
        kind: TokenKind,
        original: String,
        override_name: Option<String>,
        default_selected: bool,
    ) -> Self {
        Self {
            span,
            kind,
            original,
            override_name,
            default_selected,
        }
    }
}

/// Render `raw` with each accepted span replaced by `<@name>`. Overlapping
/// later spans are dropped; surrounding text (including original quote marks
/// outside the span) is preserved verbatim.
pub fn render_with_placeholders(raw: &str, accepted: &[(Span, String)]) -> String {
    let mut ordered: Vec<(Span, &str)> = accepted.iter().map(|(s, n)| (*s, n.as_str())).collect();
    ordered.sort_by_key(|(s, _)| (s.start, s.end));

    let mut out = String::with_capacity(raw.len());
    let mut cursor = 0;
    for (span, name) in ordered {
        if span.start < cursor || span.end > raw.len() || span.start > span.end {
            continue;
        }
        out.push_str(&raw[cursor..span.start]);
        out.push_str("<@");
        out.push_str(name);
        out.push('>');
        cursor = span.end;
    }
    out.push_str(&raw[cursor..]);
    out
}

fn tokenize_with_spans(raw: &str) -> Vec<(Span, String)> {
    if shell_words::split(raw).is_err() {
        return whitespace_split_with_spans(raw);
    }
    // Re-scan the raw string ourselves; the byte-faithful raw substrings keep
    // span math accurate even when shell-words would have unescaped them.
    whitespace_split_with_spans(raw)
}

fn whitespace_split_with_spans(raw: &str) -> Vec<(Span, String)> {
    let bytes = raw.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        let start = i;
        let mut quote: Option<u8> = None;
        while i < bytes.len() {
            let b = bytes[i];
            match quote {
                Some(q) => {
                    if b == q {
                        quote = None;
                    }
                    i += 1;
                }
                None => {
                    if b.is_ascii_whitespace() {
                        break;
                    }
                    if b == b'\'' || b == b'"' {
                        quote = Some(b);
                    }
                    i += 1;
                }
            }
        }
        out.push((Span { start, end: i }, raw[start..i].to_string()));
    }
    out
}

fn split_flag_eq(text: &str) -> Option<(&str, usize)> {
    if !text.starts_with("--") {
        return None;
    }
    let idx = text.find('=')?;
    Some((&text[..idx], idx + 1))
}

fn is_secret_flag(flag: &str) -> bool {
    let lower = flag.to_ascii_lowercase();
    matches!(
        lower.as_str(),
        "--token" | "--password" | "--api-key" | "--apikey" | "--secret"
    )
}

fn is_likely_base64_secret(text: &str) -> bool {
    if text.len() < 24 {
        return false;
    }
    let mut has_alpha = false;
    let mut has_digit = false;
    for c in text.chars() {
        match c {
            'A'..='Z' | 'a'..='z' => has_alpha = true,
            '0'..='9' => has_digit = true,
            '+' | '/' | '=' | '_' | '-' => {}
            _ => return false,
        }
    }
    has_alpha && has_digit
}

fn classify_token(text: &str) -> Option<TokenKind> {
    if text.is_empty() {
        return None;
    }
    if is_url(text) {
        return Some(TokenKind::Url);
    }
    if is_ipv4(text) {
        return Some(TokenKind::Ipv4);
    }
    if is_ipv6_literal(text) {
        return Some(TokenKind::Ipv6);
    }
    if is_quoted_string(text) {
        return Some(TokenKind::QuotedString);
    }
    if is_key_value(text) {
        return Some(TokenKind::KeyValue);
    }
    if is_abs_path(text) {
        return Some(TokenKind::Path);
    }
    if is_semver(text) {
        return Some(TokenKind::Semver);
    }
    if is_long_hex(text) {
        return Some(TokenKind::LongHex);
    }
    None
}

fn is_url(text: &str) -> bool {
    text.starts_with("http://") || text.starts_with("https://")
}

fn is_ipv4(text: &str) -> bool {
    let head = text.split(':').next().unwrap_or(text);
    let parts: Vec<&str> = head.split('.').collect();
    if parts.len() != 4 {
        return false;
    }
    parts.iter().all(|p| {
        !p.is_empty()
            && p.len() <= 3
            && p.chars().all(|c| c.is_ascii_digit())
            && p.parse::<u8>().is_ok()
    })
}

fn is_ipv6_literal(text: &str) -> bool {
    let inner = text.trim_start_matches('[').trim_end_matches(']');
    if inner.matches(':').count() < 2 {
        return false;
    }
    let mut has_hex = false;
    for c in inner.chars() {
        match c {
            '0'..='9' | ':' => {}
            'a'..='f' | 'A'..='F' => has_hex = true,
            _ => return false,
        }
    }
    has_hex
}

fn is_quoted_string(text: &str) -> bool {
    text.len() >= 2
        && ((text.starts_with('\'') && text.ends_with('\''))
            || (text.starts_with('"') && text.ends_with('"')))
}

fn is_abs_path(text: &str) -> bool {
    if text.starts_with('/') {
        return text.len() > 1 && !text.contains(' ');
    }
    if let Some(rest) = text.strip_prefix("~/") {
        return !rest.is_empty();
    }
    if let Some(rest) = text.strip_prefix("./") {
        return !rest.is_empty();
    }
    false
}

fn is_semver(text: &str) -> bool {
    let trimmed = text.trim_start_matches('v');
    let parts: Vec<&str> = trimmed.split('.').collect();
    if !(2..=3).contains(&parts.len()) {
        return false;
    }
    parts
        .iter()
        .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

fn is_key_value(text: &str) -> bool {
    let Some((key, value)) = text.split_once('=') else {
        return false;
    };
    if key.is_empty() || value.is_empty() {
        return false;
    }
    if key.starts_with('-') {
        return false;
    }
    let first = key.chars().next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_') {
        return false;
    }
    key.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
}

fn is_long_hex(text: &str) -> bool {
    text.len() >= 8 && text.chars().all(|c| c.is_ascii_hexdigit())
}

fn name_for_kind(kind: TokenKind) -> String {
    match kind {
        TokenKind::Ipv4 | TokenKind::Ipv6 => "host",
        TokenKind::Url => "url",
        TokenKind::Path => "path",
        TokenKind::QuotedString => "value",
        TokenKind::Semver => "version",
        TokenKind::LongHex => "id",
        TokenKind::Secret => "secret",
        TokenKind::KeyValue => "value",
    }
    .to_string()
}

fn slug_lower(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut last_underscore = true;
    for c in input.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
            last_underscore = false;
        } else if !last_underscore {
            out.push('_');
            last_underscore = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    if out.is_empty() {
        "value".to_string()
    } else {
        out
    }
}

/// Bump `base` so it is unique among entries already inserted into `seen`.
/// First use returns `base`; subsequent uses get a numeric suffix
/// (`host2`, `host3`, ...).
pub fn bump_name(base: &str, seen: &mut HashMap<String, usize>) -> String {
    let count = seen.entry(base.to_string()).or_insert(0);
    *count += 1;
    if *count == 1 {
        base.to_string()
    } else {
        format!("{base}{}", *count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_url_and_path_and_semver() {
        let raw = "curl https://example.com/foo /etc/hosts v1.2.3";
        let cands = detect_variables(raw);
        let kinds: Vec<TokenKind> = cands.iter().map(|c| c.kind).collect();
        assert!(kinds.contains(&TokenKind::Url));
        assert!(kinds.contains(&TokenKind::Path));
        assert!(kinds.contains(&TokenKind::Semver));
    }

    #[test]
    fn key_value_keeps_key_literal_and_uses_key_as_name() {
        let raw = "FOO=bar baz";
        let cands = detect_variables(raw);
        let foo = cands.iter().find(|c| c.suggested_name == "foo").unwrap();
        assert_eq!(foo.original, "bar");
        assert_eq!(foo.kind, TokenKind::KeyValue);
        let body = render_with_placeholders(raw, &[(foo.span, foo.suggested_name.clone())]);
        assert_eq!(body, "FOO=<@foo> baz");
    }

    #[test]
    fn flag_eq_secret_is_default_selected() {
        let raw = "curl --token=abc123XYZdef456ghijk https://example.com";
        let cands = detect_variables(raw);
        let secret = cands
            .iter()
            .find(|c| c.kind == TokenKind::Secret)
            .expect("secret detected");
        assert!(secret.default_selected);
        assert_eq!(secret.suggested_name, "secret");
    }

    #[test]
    fn split_form_secret_takes_next_token() {
        let raw = "curl --password hunter2hunter2hunter2hunter2";
        let cands = detect_variables(raw);
        assert!(cands.iter().any(|c| c.kind == TokenKind::Secret));
    }

    #[test]
    fn authorization_bearer_value_is_secret() {
        let raw = "curl -H Authorization: Bearer abcdef1234567890abcdef1234";
        let cands = detect_variables(raw);
        assert!(cands.iter().any(|c| c.kind == TokenKind::Secret));
    }

    #[test]
    fn quoted_string_replaces_inner_keeps_outer_quotes() {
        let raw = "echo 'hello world'";
        let cands = detect_variables(raw);
        let qs = cands
            .iter()
            .find(|c| c.kind == TokenKind::QuotedString)
            .unwrap();
        let body = render_with_placeholders(raw, &[(qs.span, qs.suggested_name.clone())]);
        assert_eq!(body, "echo '<@value>'");
    }

    #[test]
    fn name_collisions_get_suffixed() {
        let raw = "ssh 10.0.0.1 10.0.0.2";
        let cands = detect_variables(raw);
        let hosts: Vec<&str> = cands
            .iter()
            .filter(|c| c.kind == TokenKind::Ipv4)
            .map(|c| c.suggested_name.as_str())
            .collect();
        assert_eq!(hosts, vec!["host", "host2"]);
    }

    #[test]
    fn overlap_drops_later_candidate() {
        let raw = "abc";
        let body = render_with_placeholders(
            raw,
            &[
                (Span { start: 0, end: 3 }, "a".to_string()),
                (Span { start: 1, end: 2 }, "b".to_string()),
            ],
        );
        assert_eq!(body, "<@a>");
    }

    #[test]
    fn unbalanced_quotes_do_not_panic() {
        let raw = "echo \"unterminated";
        let _cands = detect_variables(raw);
    }

    #[test]
    fn renderer_preserves_surrounding_text() {
        let raw = "a b c";
        let body = render_with_placeholders(raw, &[(Span { start: 2, end: 3 }, "x".to_string())]);
        assert_eq!(body, "a <@x> c");
    }

    #[test]
    fn long_hex_classified_as_id() {
        let raw = "git checkout deadbeef0123";
        let cands = detect_variables(raw);
        assert!(cands.iter().any(|c| c.suggested_name == "id"));
    }
}
