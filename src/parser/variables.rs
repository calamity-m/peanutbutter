use crate::domain::{Variable, VariableSource};

/// Extract all `<@name[:source]>` placeholders from a snippet body in order
/// of first appearance. Malformed or unterminated placeholders are silently
/// skipped. Duplicates are preserved here; callers that need unique variables
/// should use [`crate::execute::prompt::unique_variables`].
pub fn parse_variables(body: &str) -> Vec<Variable> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'<' && bytes[i + 1] == b'@' {
            let start = i + 2;
            if let Some(end) = find_placeholder_end(body, start) {
                let inner = &body[start..end];
                if let Some(var) = parse_variable_inner(inner) {
                    out.push(var);
                }
                i = end + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Scan forward from `start` in `src` to find the `>` that closes a `<@...>`
/// placeholder, treating nested `<#...>` references as opaque (their inner
/// `>` does not terminate the outer placeholder). Backslash-escaped `\<#`
/// inside the inner text is also skipped. Returns the byte index of the
/// closing `>` if found.
pub(crate) fn find_placeholder_end(src: &str, start: usize) -> Option<usize> {
    let bytes = src.as_bytes();
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == b'\\' && i + 2 < bytes.len() && bytes[i + 1] == b'<' && bytes[i + 2] == b'#'
        {
            // Skip past the escaped `\<#...>`: find its `>` and continue after.
            if let Some(offset) = src[i + 1..].find('>') {
                i = i + 1 + offset + 1;
                continue;
            }
            return None;
        }
        if bytes[i] == b'<' && i + 1 < bytes.len() && bytes[i + 1] == b'#' {
            if let Some(offset) = src[i + 2..].find('>') {
                i = i + 2 + offset + 1;
                continue;
            }
            return None;
        }
        if bytes[i] == b'>' {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn parse_variable_inner(inner: &str) -> Option<Variable> {
    let (name, source) = match inner.split_once(':') {
        Some((name, rest)) => {
            let source = if let Some(default) = rest.strip_prefix('?') {
                let template =
                    crate::syntax::parse_command_template(default).unwrap_or_else(|_| {
                        vec![crate::syntax::Fragment::Literal(default.to_string())]
                    });
                VariableSource::Default(template)
            } else if let Some(hint) = rest.strip_prefix('@') {
                // Checked before the command fallback so `<@name:@hint>` is a
                // hint, not a suggestion command named `@hint`.
                VariableSource::Hint(hint.to_string())
            } else {
                VariableSource::Command(rest.to_string())
            };
            (name.trim(), source)
        }
        None => (inner.trim(), VariableSource::Free),
    };
    if name.is_empty() || !name.chars().all(is_name_char) {
        return None;
    }
    Some(Variable {
        name: name.to_string(),
        source,
    })
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}
