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
                // Deprecated hints must remain reserved during the compatibility
                // period rather than falling through to executable commands.
                VariableSource::Hint(hint.to_string())
            } else if let Some(list) = parse_suggestion_list(rest) {
                // Checked before the command fallback: a source shaped like
                // `[ ... ]` is a static list, never a shell test.
                VariableSource::Suggestions(list.items)
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

/// A static inline suggestion list parsed from a `<@name:[a,b,c]>` source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SuggestionList {
    /// Unescaped items with empty items dropped: what the prompt offers, even
    /// when [`Self::error`] is set. Plain items are trimmed; quoted items keep
    /// their exact text.
    pub items: Vec<String>,
    /// First authoring problem found, for lint. Runtime ignores it.
    pub error: Option<SuggestionListError>,
}

impl SuggestionList {
    fn note(&mut self, problem: SuggestionListError) {
        self.error.get_or_insert(problem);
    }

    fn push_item(&mut self, current: &mut String, quoted: bool) {
        let item = if quoted {
            current.as_str()
        } else {
            current.trim()
        };
        if item.is_empty() {
            self.note(SuggestionListError::EmptyItem);
        } else {
            self.items.push(item.to_string());
        }
        current.clear();
    }
}

/// Authoring problems in an inline suggestion list. None of these stop the
/// snippet from running; `pb lint` reports them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SuggestionListError {
    /// `[]` - the prompt would offer nothing.
    Empty,
    /// `[a,,b]` / `[a,]` / `[""]` - an item is blank.
    EmptyItem,
    /// `[a]b]` - an unescaped, unquoted `]` before the closing bracket.
    StrayBracket,
    /// `[a, "b]` - a quoted item is never closed.
    UnterminatedQuote,
    /// `["b"c]` - text between a closing quote and the next `,`.
    TextAfterQuote,
    /// `[<#r>a]` - `<#` is reserved inside lists, quoted or not; items are
    /// never expanded.
    ReservedReference,
}

impl std::fmt::Display for SuggestionListError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SuggestionListError::Empty => write!(f, "suggestion list is empty"),
            SuggestionListError::EmptyItem => write!(f, "suggestion list has an empty item"),
            SuggestionListError::StrayBracket => {
                write!(f, "suggestion list has an unescaped ']' before its end")
            }
            SuggestionListError::UnterminatedQuote => {
                write!(f, "suggestion list has an unterminated quoted item")
            }
            SuggestionListError::TextAfterQuote => {
                write!(f, "suggestion list has text after a closing quote")
            }
            SuggestionListError::ReservedReference => write!(
                f,
                "suggestion list items cannot contain <# references; they are never expanded"
            ),
        }
    }
}

/// Where the scanner is within the current list item.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ItemState {
    /// Only whitespace so far; a `"` here opens a quoted item.
    Start,
    /// An unquoted item; trimmed when finished.
    Plain,
    /// Inside `"..."`; `,` and `]` are literal.
    Quoted,
    /// After the closing `"`; only whitespace should follow.
    AfterQuote,
}

/// Parse an inline placeholder source as a static suggestion list.
///
/// Grammar (after trimming the source):
/// - It is a list iff it starts with `[` and its last character is an
///   *unescaped* `]`. The end check pairs `\x` escapes but ignores quotes, so
///   `[a\\]` is a list (item `a\`) while `[a,b\]` and `[a, "b]"` are not.
///   Non-lists return `None` and the caller keeps treating the source as a
///   shell command, which is why `[ -d x ]` becomes a one-item list while
///   `[ -d x ] && ls x` stays a command.
/// - Items split on `,` and are trimmed.
/// - An item that *starts* with `"` (after whitespace) is quoted: `,`, `]`
///   and surrounding spaces inside the quotes are kept, and the quotes are
///   dropped. A `"` anywhere else is literal, as is `'`.
/// - Everywhere, `\,` `\]` `\"` `\\` unescape to the literal character and
///   `\<#` to a literal `<#` (matching command templates); any other `\x`
///   stays `\x` verbatim. So `[a, "b,c"]` and `[a, b\,c]` are the same list.
/// - `>` cannot appear, quoted or not: [`find_placeholder_end`] already closed
///   the placeholder at the first `>` outside a nested `<#...>`.
///
/// Parsing is lenient so a malformed list still runs: empty items are
/// dropped, a stray `]` or reserved `<#` stays literal, an unterminated quote
/// runs to the end of the list, and text after a closing quote is appended.
/// The first such problem is recorded in [`SuggestionList::error`] for lint.
pub(crate) fn parse_suggestion_list(source: &str) -> Option<SuggestionList> {
    let body = suggestion_list_body(source)?;
    let mut list = SuggestionList {
        items: Vec::new(),
        error: None,
    };
    if body.trim().is_empty() {
        list.note(SuggestionListError::Empty);
        return Some(list);
    }
    let mut current = String::new();
    let mut state = ItemState::Start;
    let mut chars = body.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.peek().copied() {
                Some(next @ (',' | ']' | '"' | '\\')) => {
                    chars.next();
                    current.push(next);
                }
                Some('<') if chars.clone().nth(1) == Some('#') => {
                    chars.next();
                    chars.next();
                    current.push_str("<#");
                }
                _ => current.push('\\'),
            }
            match state {
                ItemState::Start => state = ItemState::Plain,
                ItemState::AfterQuote => list.note(SuggestionListError::TextAfterQuote),
                ItemState::Plain | ItemState::Quoted => {}
            }
            continue;
        }
        if ch == '<' && chars.peek() == Some(&'#') {
            list.note(SuggestionListError::ReservedReference);
        }
        match (state, ch) {
            (ItemState::Quoted, '"') => state = ItemState::AfterQuote,
            (ItemState::Quoted, _) => current.push(ch),
            (_, ',') => {
                let quoted = state == ItemState::AfterQuote;
                list.push_item(&mut current, quoted);
                state = ItemState::Start;
            }
            (ItemState::Start | ItemState::AfterQuote, _) if ch.is_whitespace() => {}
            (ItemState::Start, '"') => state = ItemState::Quoted,
            (_, ']') => {
                list.note(SuggestionListError::StrayBracket);
                current.push(ch);
                state = state_after_text(state, &mut list);
            }
            (_, _) => {
                current.push(ch);
                state = state_after_text(state, &mut list);
            }
        }
    }
    if state == ItemState::Quoted {
        list.note(SuggestionListError::UnterminatedQuote);
    }
    let quoted = matches!(state, ItemState::Quoted | ItemState::AfterQuote);
    list.push_item(&mut current, quoted);
    Some(list)
}

/// State after a literal character outside quotes: text after a closing
/// quote is an error (kept, so the item stays usable).
fn state_after_text(state: ItemState, list: &mut SuggestionList) -> ItemState {
    match state {
        ItemState::AfterQuote => {
            list.note(SuggestionListError::TextAfterQuote);
            ItemState::AfterQuote
        }
        _ => ItemState::Plain,
    }
}

/// Format `items` as an inline `[...]` list source that parses back to exactly
/// `items` — the inverse of [`parse_suggestion_list`], used when moving a
/// frontmatter list inline. Items containing `,` or `]`, with surrounding
/// whitespace, or starting with `"` are quoted; `\`, `"` (inside quotes) and
/// `<#` are escaped.
///
/// Returns `None` when an item cannot be written inline: it is empty, spans
/// lines, or contains a `>` that would close the placeholder. Rather than
/// predict every interaction with placeholder scanning, the result is proven
/// by parsing it back.
pub(crate) fn format_suggestion_list(items: &[String]) -> Option<String> {
    if items.is_empty() || items.iter().any(|item| item.contains(['\n', '\r'])) {
        return None;
    }
    let mut source = String::from("[");
    for (index, item) in items.iter().enumerate() {
        if index > 0 {
            source.push(',');
        }
        let quoted = item.trim() != item || item.contains([',', ']']) || item.starts_with('"');
        if quoted {
            source.push('"');
        }
        let mut chars = item.chars().peekable();
        while let Some(ch) = chars.next() {
            match ch {
                '\\' => source.push_str("\\\\"),
                '"' if quoted => source.push_str("\\\""),
                '<' if chars.peek() == Some(&'#') => source.push_str("\\<"),
                _ => source.push(ch),
            }
        }
        if quoted {
            source.push('"');
        }
    }
    source.push(']');

    let probe = format!("<@x:{source}>");
    let end = find_placeholder_end(&probe, 2)?;
    let round_trips = end + 1 == probe.len()
        && parse_suggestion_list(&probe["<@x:".len()..end])
            .is_some_and(|list| list.error.is_none() && list.items == items);
    round_trips.then_some(source)
}

/// Return the text between `[` and the closing unescaped `]` when the trimmed
/// `source` is a suggestion list. Scans bytewise: `\` and `]` are ASCII, and
/// skipping the byte after `\` can only land inside a multi-byte char, whose
/// continuation bytes never match either.
fn suggestion_list_body(source: &str) -> Option<&str> {
    let rest = source.trim().strip_prefix('[')?;
    let bytes = rest.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b']' if i + 1 == bytes.len() => return Some(&rest[..i]),
            _ => i += 1,
        }
    }
    None
}
