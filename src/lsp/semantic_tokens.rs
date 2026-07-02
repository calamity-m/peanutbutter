//! Semantic tokens for peanutbutter placeholder syntax.
//!
//! Publishes token-level classification for `<@name:source>` placeholders and
//! `<#name>` dependent references so editors can colour the distinct parts of a
//! placeholder (name vs. default vs. command vs. dependent reference) instead of
//! relying on the surrounding code-fence grammar, which has no knowledge of the
//! placeholder DSL.
//!
//! Columns follow the same byte-offset-as-character convention used elsewhere in
//! the LSP module (snippet bodies are effectively ASCII).

use tower_lsp::lsp_types::*;

/// Token types reported to the client, in legend order. Indices into this slice
/// are used as `token_type` values in the encoded stream.
pub(super) const TOKEN_TYPES: &[SemanticTokenType] = &[
    SemanticTokenType::OPERATOR,  // 0: `<@`, `>` delimiters
    SemanticTokenType::VARIABLE,  // 1: placeholder name
    SemanticTokenType::STRING,    // 2: `:?default` text
    SemanticTokenType::FUNCTION,  // 3: `:command` body
    SemanticTokenType::PARAMETER, // 4: `<#name>` dependent reference
    SemanticTokenType::MODIFIER,  // 5: `:raw` modifier on a dependent reference
];

const TOK_OPERATOR: u32 = 0;
const TOK_VARIABLE: u32 = 1;
const TOK_STRING: u32 = 2;
const TOK_FUNCTION: u32 = 3;
const TOK_PARAMETER: u32 = 4;
const TOK_MODIFIER: u32 = 5;

/// A single classified span on one line, before delta encoding.
struct RawToken {
    line: u32,
    start: u32,
    len: u32,
    token_type: u32,
}

/// Compute the full set of semantic tokens for a snippet document.
pub(super) fn compute_semantic_tokens(content: &str) -> Option<SemanticTokensResult> {
    let mut raw = Vec::new();
    for (line_idx, line) in content.lines().enumerate() {
        tokenize_line(line_idx as u32, line, &mut raw);
    }
    Some(SemanticTokensResult::Tokens(SemanticTokens {
        result_id: None,
        data: encode(raw),
    }))
}

/// Scan a single line for placeholders and standalone dependent references.
fn tokenize_line(line_idx: u32, line: &str, out: &mut Vec<RawToken>) {
    let bytes = line.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'<' && bytes[i + 1] == b'@' {
            let end = closing_gt(bytes, i);
            emit_placeholder(line_idx, line, i, end, out);
            i = end;
        } else if bytes[i] == b'<' && bytes[i + 1] == b'#' && !is_escaped(bytes, i) {
            // A `<#…>` that is not nested inside a `<@…>` placeholder.
            let end = closing_gt(bytes, i);
            emit_dependent_ref(line_idx, line, i, end, out);
            i = end;
        } else {
            i += 1;
        }
    }
}

/// Index just past the closing `>` of a token starting at `start`, or the end of
/// the line if there is no `>` (unterminated placeholder).
fn closing_gt(bytes: &[u8], start: usize) -> usize {
    match bytes[start..].iter().position(|&b| b == b'>') {
        Some(rel) => start + rel + 1,
        None => bytes.len(),
    }
}

/// Whether the `<` at `pos` is backslash-escaped (`\<#name>` renders literally).
fn is_escaped(bytes: &[u8], pos: usize) -> bool {
    pos > 0 && bytes[pos - 1] == b'\\'
}

/// Emit tokens for a `<@name>` / `<@name:source>` placeholder. `start` is the
/// `<` index; `end` is one past the closing `>` (or line end if unterminated).
fn emit_placeholder(line_idx: u32, line: &str, start: usize, end: usize, out: &mut Vec<RawToken>) {
    push(out, line_idx, start, 2, TOK_OPERATOR); // `<@`

    let has_close = line.as_bytes().get(end - 1) == Some(&b'>');
    let inner_start = start + 2;
    let inner_end = if has_close { end - 1 } else { end };
    if inner_start >= inner_end {
        if has_close {
            push(out, line_idx, inner_end, 1, TOK_OPERATOR);
        }
        return;
    }
    let inner = &line[inner_start..inner_end];

    match inner.find(':') {
        None => {
            push(
                out,
                line_idx,
                inner_start,
                (inner_end - inner_start) as u32,
                TOK_VARIABLE,
            );
        }
        Some(colon_rel) => {
            let name_end = inner_start + colon_rel;
            if name_end > inner_start {
                push(
                    out,
                    line_idx,
                    inner_start,
                    (name_end - inner_start) as u32,
                    TOK_VARIABLE,
                );
            }
            // Source region includes the `:` separator. A leading `?` marks a
            // default and a leading `@` marks a hint (both display/prefill
            // text, so string); anything else is a suggestion command
            // (function).
            let is_text = matches!(line.as_bytes().get(name_end + 1), Some(&b'?') | Some(&b'@'));
            let base = if is_text { TOK_STRING } else { TOK_FUNCTION };
            emit_source_region(line_idx, line, name_end, inner_end, base, out);
        }
    }

    if has_close {
        push(out, line_idx, inner_end, 1, TOK_OPERATOR); // `>`
    }
}

/// Emit `base`-typed tokens across `[region_start, region_end)`, splitting around
/// nested `<#…>` dependent references which get their own parameter/modifier
/// tokens.
fn emit_source_region(
    line_idx: u32,
    line: &str,
    region_start: usize,
    region_end: usize,
    base: u32,
    out: &mut Vec<RawToken>,
) {
    let bytes = line.as_bytes();
    let mut cursor = region_start;
    let mut i = region_start;
    while i + 1 < region_end {
        if bytes[i] == b'<' && bytes[i + 1] == b'#' && !is_escaped(bytes, i) {
            if i > cursor {
                push(out, line_idx, cursor, (i - cursor) as u32, base);
            }
            let ref_end = closing_gt(bytes, i).min(region_end);
            emit_dependent_ref(line_idx, line, i, ref_end, out);
            cursor = ref_end;
            i = ref_end;
        } else {
            i += 1;
        }
    }
    if cursor < region_end {
        push(out, line_idx, cursor, (region_end - cursor) as u32, base);
    }
}

/// Emit tokens for a `<#name>` / `<#name:raw>` dependent reference: a parameter
/// token over `<#name` and a modifier token over a trailing `:modifier`.
fn emit_dependent_ref(
    line_idx: u32,
    line: &str,
    start: usize,
    end: usize,
    out: &mut Vec<RawToken>,
) {
    let has_close = line.as_bytes().get(end - 1) == Some(&b'>');
    let inner_end = if has_close { end - 1 } else { end };
    if start + 2 >= inner_end {
        return;
    }
    let inner = &line[start + 2..inner_end];
    match inner.find(':') {
        None => push(
            out,
            line_idx,
            start,
            (inner_end - start) as u32,
            TOK_PARAMETER,
        ),
        Some(colon_rel) => {
            let colon_abs = start + 2 + colon_rel;
            push(
                out,
                line_idx,
                start,
                (colon_abs - start) as u32,
                TOK_PARAMETER,
            );
            if inner_end > colon_abs {
                push(
                    out,
                    line_idx,
                    colon_abs,
                    (inner_end - colon_abs) as u32,
                    TOK_MODIFIER,
                );
            }
        }
    }
}

fn push(out: &mut Vec<RawToken>, line: u32, start: usize, len: u32, token_type: u32) {
    if len == 0 {
        return;
    }
    out.push(RawToken {
        line,
        start: start as u32,
        len,
        token_type,
    });
}

/// Delta-encode raw tokens into the LSP wire format. Tokens are sorted by
/// position; each is encoded relative to the previous one.
fn encode(mut raw: Vec<RawToken>) -> Vec<SemanticToken> {
    raw.sort_by_key(|t| (t.line, t.start));
    let mut data = Vec::with_capacity(raw.len());
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;
    for t in raw {
        let delta_line = t.line - prev_line;
        let delta_start = if delta_line == 0 {
            t.start - prev_start
        } else {
            t.start
        };
        data.push(SemanticToken {
            delta_line,
            delta_start,
            length: t.len,
            token_type: t.token_type,
            token_modifiers_bitset: 0,
        });
        prev_line = t.line;
        prev_start = t.start;
    }
    data
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Decode the delta stream back into absolute `(line, start, len, type)`
    /// tuples for easier assertions.
    fn absolute(content: &str) -> Vec<(u32, u32, u32, u32)> {
        let SemanticTokensResult::Tokens(tokens) = compute_semantic_tokens(content).unwrap() else {
            panic!("expected token result");
        };
        let mut out = Vec::new();
        let mut line = 0u32;
        let mut start = 0u32;
        for t in tokens.data {
            if t.delta_line == 0 {
                start += t.delta_start;
            } else {
                line += t.delta_line;
                start = t.delta_start;
            }
            out.push((line, start, t.length, t.token_type));
        }
        out
    }

    #[test]
    fn free_form_placeholder() {
        // `<@name>` → operator `<@`, variable `name`, operator `>`.
        let toks = absolute("<@name>");
        assert_eq!(
            toks,
            vec![
                (0, 0, 2, TOK_OPERATOR),
                (0, 2, 4, TOK_VARIABLE),
                (0, 6, 1, TOK_OPERATOR),
            ]
        );
    }

    #[test]
    fn default_placeholder_is_string() {
        // `<@path:?.>` → name `path`, default `?.` typed as string.
        let toks = absolute("<@path:?.>");
        assert!(toks.contains(&(0, 2, 4, TOK_VARIABLE)));
        // `:?.` spans columns 6..9.
        assert!(toks.contains(&(0, 6, 3, TOK_STRING)));
    }

    #[test]
    fn hint_placeholder_is_string() {
        // `<@input:@hello>` → name `input`, hint `@hello` typed as string.
        let toks = absolute("<@input:@hello>");
        assert!(toks.contains(&(0, 2, 5, TOK_VARIABLE)));
        // `:@hello` spans columns 7..14.
        assert!(toks.contains(&(0, 7, 7, TOK_STRING)));
    }

    #[test]
    fn command_placeholder_is_function() {
        // `<@file:rg . --files>` → command body typed as function.
        let toks = absolute("<@file:rg . --files>");
        assert!(toks.contains(&(0, 2, 4, TOK_VARIABLE)));
        // `:rg . --files` spans columns 6..19.
        assert!(toks.contains(&(0, 6, 13, TOK_FUNCTION)));
    }

    #[test]
    fn nested_dependent_ref_splits_command() {
        // `<@key:ls <#bucket>>`: command base around the nested `<#bucket>` ref.
        let toks = absolute("<@key:ls <#bucket>>");
        assert!(toks.contains(&(0, 2, 3, TOK_VARIABLE))); // key
        assert!(toks.contains(&(0, 5, 4, TOK_FUNCTION))); // `:ls ` before ref
        assert!(toks.contains(&(0, 9, 8, TOK_PARAMETER))); // `<#bucket`
    }

    #[test]
    fn raw_modifier_on_nested_ref() {
        // `<@out:?<#name:raw>>`: default with a raw dependent reference.
        let toks = absolute("<@out:?<#name:raw>>");
        assert!(toks.contains(&(0, 2, 3, TOK_VARIABLE))); // out
        assert!(toks.contains(&(0, 5, 2, TOK_STRING))); // `:?` before ref
        assert!(toks.contains(&(0, 7, 6, TOK_PARAMETER))); // `<#name`
        assert!(toks.contains(&(0, 13, 4, TOK_MODIFIER))); // `:raw`
    }

    #[test]
    fn escaped_dependent_ref_is_not_tokenized() {
        let toks = absolute(r"echo \<#name>");
        assert!(toks.is_empty());
    }

    #[test]
    fn multiline_delta_encoding() {
        let toks = absolute("<@a>\n<@b>");
        assert!(toks.contains(&(0, 0, 2, TOK_OPERATOR)));
        assert!(toks.contains(&(1, 0, 2, TOK_OPERATOR)));
        assert!(toks.contains(&(1, 2, 1, TOK_VARIABLE)));
    }
}
