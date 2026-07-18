use crate::domain::{Frontmatter, VariableSpec};
use std::collections::BTreeMap;

pub(super) fn parse_frontmatter(lines: &[&str]) -> (Frontmatter, usize) {
    if lines.first().map(|l| l.trim()) != Some("---") {
        return (Frontmatter::default(), 0);
    }
    let mut end_idx = None;
    for (i, line) in lines.iter().enumerate().skip(1) {
        if line.trim() == "---" {
            end_idx = Some(i);
            break;
        }
    }
    let end = match end_idx {
        Some(i) => i,
        None => return (Frontmatter::default(), 0),
    };
    let fm = parse_yaml_frontmatter(&lines[1..end]);
    (fm, end + 1)
}

fn parse_yaml_frontmatter(lines: &[&str]) -> Frontmatter {
    let mut fm = Frontmatter::default();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }
        let (key, value) = match trimmed.split_once(':') {
            Some(kv) => kv,
            None => {
                i += 1;
                continue;
            }
        };
        let key = key.trim();
        let value = value.trim();

        if value.starts_with('[') && value.ends_with(']') {
            if key == "tags" {
                fm.tags = parse_inline_list(value);
            }
            i += 1;
            continue;
        }

        if value.is_empty() {
            i += 1;
            match key {
                "tags" => {
                    while i < lines.len() {
                        let child = lines[i].trim_start();
                        let indent = lines[i].len() - child.len();
                        if indent == 0 || !child.starts_with('-') {
                            break;
                        }
                        fm.tags.push(strip_quotes(child[1..].trim()));
                        i += 1;
                    }
                }
                "variables" => {
                    let (vars, consumed) = parse_variable_block(&lines[i..]);
                    fm.variables = vars;
                    i += consumed;
                }
                _ => {}
            }
            continue;
        }

        match key {
            "name" => fm.name = Some(strip_quotes(value)),
            "description" => fm.description = Some(strip_quotes(value)),
            _ => {}
        }
        i += 1;
    }

    fm
}

/// Parse a `variables:` block into a map of name → [`VariableSpec`].
/// Returns the map and the number of lines consumed from `lines`.
fn parse_variable_block(lines: &[&str]) -> (BTreeMap<String, VariableSpec>, usize) {
    let mut out = BTreeMap::new();
    let mut j = 0;

    while j < lines.len() {
        let line = lines[j];
        let trimmed = line.trim_start();
        let var_indent = line.len() - trimmed.len();

        if var_indent == 0 || trimmed.is_empty() || trimmed.starts_with('#') {
            break;
        }

        let (name, rest) = match trimmed.split_once(':') {
            Some(kv) => kv,
            None => {
                j += 1;
                continue;
            }
        };
        let name = name.trim().to_string();
        let rest = rest.trim();

        // A valid variable entry is a block mapping (`varname:` with no inline value).
        if !rest.is_empty() {
            j += 1;
            continue;
        }

        j += 1;
        let mut spec = VariableSpec::default();
        while j < lines.len() {
            let field_line = lines[j];
            let field_trim = field_line.trim_start();
            let field_indent = field_line.len() - field_trim.len();
            if field_indent <= var_indent {
                break;
            }
            if let Some((fkey, fval)) = field_trim.split_once(':') {
                let fkey = fkey.trim();
                let fval = fval.trim();
                if !fval.is_empty() {
                    match fkey {
                        "default" => spec.default = Some(strip_quotes(fval)),
                        "default_value" => spec.default_value = Some(strip_quotes(fval)),
                        "suggestions" if fval.starts_with('[') && fval.ends_with(']') => {
                            spec.suggestions = parse_inline_list(fval);
                        }
                        "command" => spec.command = Some(strip_quotes(fval)),
                        "hint" => spec.hint = Some(strip_quotes(fval)),
                        _ => {}
                    }
                } else if fkey == "suggestions" {
                    j += 1;
                    while j < lines.len() {
                        let item_line = lines[j];
                        let item_trim = item_line.trim_start();
                        let item_indent = item_line.len() - item_trim.len();
                        if item_indent <= field_indent || !item_trim.starts_with('-') {
                            break;
                        }
                        let suggestion = strip_quotes(item_trim[1..].trim());
                        if !suggestion.is_empty() {
                            spec.suggestions.push(suggestion);
                        }
                        j += 1;
                    }
                    continue;
                }
            }
            j += 1;
        }
        out.insert(name, spec);
    }

    (out, j)
}

fn parse_inline_list(value: &str) -> Vec<String> {
    let inner = &value[1..value.len() - 1];
    inner
        .split(',')
        .map(|s| strip_quotes(s.trim()))
        .filter(|s| !s.is_empty())
        .collect()
}

fn strip_quotes(s: &str) -> String {
    let bytes = s.as_bytes();
    if bytes.len() >= 2 {
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if first == b'"' && last == b'"' {
            return s[1..s.len() - 1]
                .replace("\\\"", "\"")
                .replace("\\\\", "\\");
        }
        if first == b'\'' && last == b'\'' {
            return s[1..s.len() - 1].replace("''", "'");
        }
    }
    s.to_string()
}
