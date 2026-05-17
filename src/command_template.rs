//! Parsing and rendering of suggestion-command templates with `<#name>` /
//! `<#name:raw>` dependent-variable references.
//!
//! A suggestion command may reference values the user has already confirmed
//! for earlier variables. Dependence is explicit: a `<#bucket>` token expands
//! to the shell-single-quoted form of the confirmed value for `bucket`; a
//! `<#bucket:raw>` token splices the value verbatim (no quoting).
//!
//! Literal `<#...>` text can be written by escaping the opener as `\<#...>`.

use std::collections::BTreeSet;
use std::fmt;

/// One piece of a parsed suggestion-command template.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fragment {
    /// Verbatim text. Backslash escapes for `\<#` have already been collapsed.
    Literal(String),
    /// Dependent reference to a previously-confirmed variable.
    Ref {
        /// Upstream variable name.
        name: String,
        /// If true, splice the confirmed value verbatim; else shell-single-quote.
        raw: bool,
    },
}

/// Parsed suggestion-command template: an ordered sequence of literal runs
/// and dependent `<#name>` / `<#name:raw>` references.
pub type CommandTemplate = Vec<Fragment>;

/// Failure to parse `<#...>` syntax in a command source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    /// `<#` was opened but never closed by `>`.
    UnterminatedRef,
    /// `<#...>` had an empty or otherwise invalid name.
    InvalidName(String),
    /// `<#name:foo>` used an unknown modifier (only `raw` is supported).
    UnknownModifier(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::UnterminatedRef => write!(f, "unterminated <# reference"),
            ParseError::InvalidName(name) => write!(f, "invalid <# reference name '{name}'"),
            ParseError::UnknownModifier(modifier) => {
                write!(f, "unknown <# reference modifier ':{modifier}'")
            }
        }
    }
}

impl std::error::Error for ParseError {}

/// Failure to render a parsed template into a final command string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderError {
    /// A `<#name>` reference had no confirmed upstream value.
    MissingConfirmed(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::MissingConfirmed(name) => {
                write!(f, "dependent variable '{name}' has not been confirmed")
            }
        }
    }
}

impl std::error::Error for RenderError {}

/// Tokenize `src` into a `Vec<Fragment>`. Backslash escapes (`\<#...>`) are
/// collapsed into literal text without producing a `Ref`. Malformed `<#...>`
/// returns an error.
pub fn parse_command_template(src: &str) -> Result<CommandTemplate, ParseError> {
    let mut out: CommandTemplate = Vec::new();
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut literal = String::new();
    while i < bytes.len() {
        // Escaped opener: \<# ... > renders as literal <# ... >
        if bytes[i] == b'\\' && i + 2 < bytes.len() && bytes[i + 1] == b'<' && bytes[i + 2] == b'#'
        {
            // Find matching `>` and treat the whole thing as literal (without the leading backslash).
            let start = i + 1;
            match src[start..].find('>') {
                Some(offset) => {
                    let end = start + offset + 1;
                    literal.push_str(&src[start..end]);
                    i = end;
                    continue;
                }
                None => {
                    // No closing `>` — treat the backslash literally and continue.
                    literal.push('\\');
                    i += 1;
                    continue;
                }
            }
        }

        if bytes[i] == b'<' && i + 1 < bytes.len() && bytes[i + 1] == b'#' {
            let inner_start = i + 2;
            let Some(offset) = src[inner_start..].find('>') else {
                return Err(ParseError::UnterminatedRef);
            };
            let inner_end = inner_start + offset;
            let inner = &src[inner_start..inner_end];
            let (name, raw) = parse_ref_inner(inner)?;
            if !literal.is_empty() {
                out.push(Fragment::Literal(std::mem::take(&mut literal)));
            }
            out.push(Fragment::Ref {
                name: name.to_string(),
                raw,
            });
            i = inner_end + 1;
            continue;
        }

        // Default: consume one byte (the source is UTF-8, but we operate
        // bytewise and copy substrings, which preserves char boundaries
        // because `<`, `#`, `>`, `\` are all ASCII).
        let ch = src[i..].chars().next().expect("char boundary");
        literal.push(ch);
        i += ch.len_utf8();
    }
    if !literal.is_empty() {
        out.push(Fragment::Literal(literal));
    }
    Ok(out)
}

fn parse_ref_inner(inner: &str) -> Result<(&str, bool), ParseError> {
    let (name, raw) = match inner.split_once(':') {
        Some((name, modifier)) => {
            if modifier == "raw" {
                (name, true)
            } else {
                return Err(ParseError::UnknownModifier(modifier.to_string()));
            }
        }
        None => (inner, false),
    };
    let name = name.trim();
    if name.is_empty() || !name.chars().all(is_name_char) {
        return Err(ParseError::InvalidName(name.to_string()));
    }
    Ok((name, raw))
}

fn is_name_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// Return the set of upstream variable names referenced by `template`.
pub fn referenced_names(template: &CommandTemplate) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for fragment in template {
        if let Fragment::Ref { name, .. } = fragment {
            out.insert(name.clone());
        }
    }
    out
}

/// Returns `true` if `template` references any upstream variable.
pub fn is_dependent(template: &CommandTemplate) -> bool {
    template.iter().any(|f| matches!(f, Fragment::Ref { .. }))
}

/// Render `template` into a final command string using `confirmed` values.
/// `<#name>` references are shell-single-quoted; `<#name:raw>` are spliced
/// verbatim. Missing confirmed values yield `RenderError::MissingConfirmed`.
pub fn render(
    template: &CommandTemplate,
    confirmed: &std::collections::BTreeMap<String, String>,
) -> Result<String, RenderError> {
    let mut out = String::new();
    for fragment in template {
        match fragment {
            Fragment::Literal(text) => out.push_str(text),
            Fragment::Ref { name, raw: false } => {
                let value = confirmed
                    .get(name)
                    .ok_or_else(|| RenderError::MissingConfirmed(name.clone()))?;
                out.push_str(&shell_single_quote(value));
            }
            Fragment::Ref { name, raw: true } => {
                let value = confirmed
                    .get(name)
                    .ok_or_else(|| RenderError::MissingConfirmed(name.clone()))?;
                out.push_str(value);
            }
        }
    }
    Ok(out)
}

/// Wrap `value` in POSIX single quotes, escaping any embedded `'` as `'\''`.
pub fn shell_single_quote(value: &str) -> String {
    let mut out = String::with_capacity(value.len() + 2);
    out.push('\'');
    for ch in value.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn lit(s: &str) -> Fragment {
        Fragment::Literal(s.to_string())
    }
    fn r(name: &str) -> Fragment {
        Fragment::Ref {
            name: name.to_string(),
            raw: false,
        }
    }
    fn rraw(name: &str) -> Fragment {
        Fragment::Ref {
            name: name.to_string(),
            raw: true,
        }
    }

    #[test]
    fn parses_literal_only() {
        let t = parse_command_template("echo hi").unwrap();
        assert_eq!(t, vec![lit("echo hi")]);
    }

    #[test]
    fn parses_simple_ref() {
        let t = parse_command_template("aws s3 ls s3://<#bucket>/").unwrap();
        assert_eq!(t, vec![lit("aws s3 ls s3://"), r("bucket"), lit("/")]);
    }

    #[test]
    fn parses_raw_ref() {
        let t = parse_command_template("kubectl <#verb:raw> -o name").unwrap();
        assert_eq!(t, vec![lit("kubectl "), rraw("verb"), lit(" -o name")]);
    }

    #[test]
    fn escaped_opener_is_literal() {
        let t = parse_command_template(r"echo \<#bucket>").unwrap();
        assert_eq!(t, vec![lit("echo <#bucket>")]);
        assert!(referenced_names(&t).is_empty());
    }

    #[test]
    fn unterminated_ref_errors() {
        assert!(matches!(
            parse_command_template("echo <#bucket"),
            Err(ParseError::UnterminatedRef)
        ));
    }

    #[test]
    fn unknown_modifier_errors() {
        assert!(matches!(
            parse_command_template("echo <#x:nope>"),
            Err(ParseError::UnknownModifier(_))
        ));
    }

    #[test]
    fn invalid_name_errors() {
        assert!(matches!(
            parse_command_template("echo <#>"),
            Err(ParseError::InvalidName(_))
        ));
        assert!(matches!(
            parse_command_template("echo <#bad name>"),
            Err(ParseError::InvalidName(_))
        ));
    }

    #[test]
    fn shell_single_quote_escapes_apostrophe() {
        assert_eq!(shell_single_quote("foo"), "'foo'");
        assert_eq!(shell_single_quote(""), "''");
        assert_eq!(shell_single_quote("O'Brien"), "'O'\\''Brien'");
        assert_eq!(shell_single_quote("a 'b' c"), "'a '\\''b'\\'' c'");
    }

    #[test]
    fn render_quotes_default_form() {
        let template = parse_command_template("ls <#bucket>").unwrap();
        let mut values = BTreeMap::new();
        values.insert("bucket".to_string(), "foo".to_string());
        assert_eq!(render(&template, &values).unwrap(), "ls 'foo'");

        values.insert("bucket".to_string(), "O'Brien's".to_string());
        assert_eq!(
            render(&template, &values).unwrap(),
            "ls 'O'\\''Brien'\\''s'"
        );
    }

    #[test]
    fn render_raw_is_verbatim() {
        let template = parse_command_template("kubectl <#verb:raw> -o name").unwrap();
        let mut values = BTreeMap::new();
        values.insert("verb".to_string(), "get pods".to_string());
        assert_eq!(
            render(&template, &values).unwrap(),
            "kubectl get pods -o name"
        );
    }

    #[test]
    fn render_missing_value_errors() {
        let template = parse_command_template("ls <#bucket>").unwrap();
        let values = BTreeMap::new();
        assert!(matches!(
            render(&template, &values),
            Err(RenderError::MissingConfirmed(name)) if name == "bucket"
        ));
    }

    #[test]
    fn escaped_literal_not_in_referenced_names() {
        let template = parse_command_template(r"echo \<#bucket>").unwrap();
        assert!(referenced_names(&template).is_empty());
    }

    #[test]
    fn referenced_names_collects_unique() {
        let t = parse_command_template("<#a> <#b> <#a:raw>").unwrap();
        let names = referenced_names(&t);
        assert!(names.contains("a"));
        assert!(names.contains("b"));
        assert_eq!(names.len(), 2);
    }

    #[test]
    fn quotes_value_with_spaces_and_metas() {
        let template = parse_command_template("echo <#x>").unwrap();
        let mut values = BTreeMap::new();
        values.insert("x".to_string(), "a $b; `c`".to_string());
        assert_eq!(render(&template, &values).unwrap(), "echo 'a $b; `c`'");
    }
}
