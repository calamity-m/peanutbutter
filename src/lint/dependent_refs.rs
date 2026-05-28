//! Dependent variable reference lint rules — unknown, forward, self, and
//! raw-default-untrusted-upstream checks for `<#name>` references inside
//! suggestion commands and inline defaults.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::config::VariableInputConfig;
use crate::domain::{Snippet, VariableSource, VariableSpec};
use crate::parser;
use crate::syntax::{Fragment, parse_command_template};

use super::{
    CODE_FORWARD_VARIABLE_REFERENCE, CODE_INVALID_DEPENDENT_REFERENCE,
    CODE_RAW_DEFAULT_UNTRUSTED_UPSTREAM, CODE_SELF_VARIABLE_REFERENCE,
    CODE_UNKNOWN_VARIABLE_REFERENCE, FileContext, LintFinding, LintSeverity, finding,
};

/// Walk all suggestion commands in `file` and emit lint findings for
/// `<#name>` references that are: unknown, forward, or self.
pub(super) fn lint_dependent_references(
    file: &FileContext,
    globals: &BTreeMap<String, VariableInputConfig>,
) -> Vec<LintFinding> {
    let mut out = Vec::new();

    // Pre-scan the file content for all <#name> spans so we can attach
    // precise diagnostic ranges. We pop spans as we attribute findings to
    // them — if two refs of the same name exist, each one gets a position
    // in document order.
    let mut spans = find_dependent_ref_spans(&file.content);
    let mut take_span = |name: &str| -> Option<(usize, usize, usize)> {
        let pos = spans.iter().position(|(n, ..)| n == name)?;
        let (_, l, c0, c1) = spans.remove(pos);
        Some((l, c0, c1))
    };

    for snippet in &file.parsed.snippets {
        // Build prompt order from the deduplicated body variables. Forward
        // references are decided against this order.
        let mut prompt_order: Vec<String> = Vec::new();
        let mut seen = HashSet::new();
        for variable in &snippet.variables {
            if seen.insert(variable.name.clone()) {
                prompt_order.push(variable.name.clone());
            }
        }
        let order_index: HashMap<String, usize> = prompt_order
            .iter()
            .enumerate()
            .map(|(i, name)| (name.clone(), i))
            .collect();

        // The universe of declared variables: body, frontmatter, config.
        let mut declared: HashSet<String> = prompt_order.iter().cloned().collect();
        declared.extend(file.parsed.frontmatter.variables.keys().cloned());
        declared.extend(globals.keys().cloned());
        // Builtins are always declared.
        declared.insert("file".to_string());
        declared.insert("directory".to_string());

        let mut templates = Vec::new();
        for (owner, command) in
            command_sources(snippet, &file.parsed.frontmatter.variables, globals)
        {
            match parse_command_template(&command) {
                Ok(template) => templates.push((owner, command, template, false)),
                Err(err) => out.push(
                    finding(
                        LintSeverity::Error,
                        CODE_INVALID_DEPENDENT_REFERENCE,
                        file.path.clone(),
                        None,
                        Some(snippet.id.clone()),
                        format!(
                            "suggestion command for variable '{owner}' has an invalid <# reference"
                        ),
                        Some(err.to_string()),
                    )
                    .with_suppress_command(command),
                ),
            }
        }
        for (owner, default) in default_sources(snippet) {
            match parse_command_template(&default) {
                Ok(template) => templates.push((owner, default, template, true)),
                Err(err) => out.push(
                    finding(
                        LintSeverity::Error,
                        CODE_INVALID_DEPENDENT_REFERENCE,
                        file.path.clone(),
                        None,
                        Some(snippet.id.clone()),
                        format!("default for variable '{owner}' has an invalid <# reference"),
                        Some(err.to_string()),
                    )
                    .with_suppress_command(default),
                ),
            }
        }

        for (owner, source, template, is_default) in templates {
            for fragment in &template {
                let Fragment::Ref {
                    name: ref_name,
                    raw,
                } = fragment
                else {
                    continue;
                };
                let span = take_span(ref_name);
                if ref_name == &owner {
                    let mut f = finding(
                        LintSeverity::Error,
                        CODE_SELF_VARIABLE_REFERENCE,
                        file.path.clone(),
                        None,
                        Some(snippet.id.clone()),
                        format!("variable '{owner}' references itself via <#{ref_name}>"),
                        None,
                    )
                    .with_suppress_command(source.clone());
                    if let Some((l, c0, c1)) = span {
                        f = f.with_span(l, c0, c1);
                    }
                    out.push(f);
                    continue;
                }
                if !declared.contains(ref_name) {
                    let mut f = finding(
                        LintSeverity::Error,
                        CODE_UNKNOWN_VARIABLE_REFERENCE,
                        file.path.clone(),
                        None,
                        Some(snippet.id.clone()),
                        format!("variable '{owner}' references unknown <#{ref_name}>"),
                        Some("declare it with an inline placeholder, frontmatter variable, or config override".to_string()),
                    )
                    .with_suppress_command(source.clone());
                    if let Some((l, c0, c1)) = span {
                        f = f.with_span(l, c0, c1);
                    }
                    out.push(f);
                    continue;
                }
                if let (Some(&owner_idx), Some(&ref_idx)) =
                    (order_index.get(&owner), order_index.get(ref_name))
                    && ref_idx >= owner_idx
                {
                    let mut f = finding(
                        LintSeverity::Error,
                        CODE_FORWARD_VARIABLE_REFERENCE,
                        file.path.clone(),
                        None,
                        Some(snippet.id.clone()),
                        format!(
                            "variable '{owner}' references <#{ref_name}> which comes later in prompt order"
                        ),
                        None,
                    )
                    .with_suppress_command(source.clone());
                    if let Some((l, c0, c1)) = span {
                        f = f.with_span(l, c0, c1);
                    }
                    out.push(f);
                    continue;
                }
                if is_default
                    && *raw
                    && is_free_form_upstream(
                        ref_name,
                        snippet,
                        &file.parsed.frontmatter.variables,
                        globals,
                    )
                {
                    let mut f = finding(
                        LintSeverity::Warning,
                        CODE_RAW_DEFAULT_UNTRUSTED_UPSTREAM,
                        file.path.clone(),
                        None,
                        Some(snippet.id.clone()),
                        format!(
                            "default for variable '{owner}' raw-splices free-form upstream <#{ref_name}:raw>"
                        ),
                        Some("use <#name> for shell quoting, constrain the upstream with a default/suggestions/command, or suppress this lint if the raw splice is intentional".to_string()),
                    )
                    .with_suppress_command(source.clone());
                    if let Some((l, c0, c1)) = span {
                        f = f.with_span(l, c0, c1);
                    }
                    out.push(f);
                }
            }
        }
    }
    out
}

/// Find all `<#name>` token spans in `content`. Returns
/// `(name, line_1based, col_start, col_end)` tuples in document order.
/// Backslash-escaped `\<#...>` is skipped. The returned columns are 0-based
/// byte offsets within the line.
fn find_dependent_ref_spans(content: &str) -> Vec<(String, usize, usize, usize)> {
    let mut out = Vec::new();
    for (line_idx, line) in content.lines().enumerate() {
        let bytes = line.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            // Skip escaped \<#...>
            if bytes[i] == b'\\'
                && i + 2 < bytes.len()
                && bytes[i + 1] == b'<'
                && bytes[i + 2] == b'#'
            {
                if let Some(off) = line[i + 1..].find('>') {
                    i = i + 1 + off + 1;
                    continue;
                }
                i += 1;
                continue;
            }
            if bytes[i] == b'<' && i + 1 < bytes.len() && bytes[i + 1] == b'#' {
                let inner_start = i + 2;
                let Some(off) = line[inner_start..].find('>') else {
                    i += 1;
                    continue;
                };
                let inner_end = inner_start + off;
                let inner = &line[inner_start..inner_end];
                let name = inner.split(':').next().unwrap_or(inner).trim().to_string();
                if !name.is_empty() {
                    out.push((name, line_idx + 1, i, inner_end + 1));
                }
                i = inner_end + 1;
                continue;
            }
            i += 1;
        }
    }
    out
}

fn default_sources(snippet: &Snippet) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    let body = snippet.body.as_str();
    let bytes = body.as_bytes();
    let mut i = 0;
    while i + 1 < bytes.len() {
        if bytes[i] == b'<' && bytes[i + 1] == b'@' {
            let inner_start = i + 2;
            if let Some(inner_end) = parser::find_placeholder_end(body, inner_start) {
                let inner = &body[inner_start..inner_end];
                if let Some((name, rest)) = inner.split_once(':')
                    && let Some(default) = rest.strip_prefix('?')
                {
                    let name = name.trim();
                    if seen.insert(name.to_string()) {
                        out.push((name.to_string(), default.to_string()));
                    }
                }
                i = inner_end + 1;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn is_free_form_upstream(
    name: &str,
    snippet: &Snippet,
    locals: &BTreeMap<String, VariableSpec>,
    globals: &BTreeMap<String, VariableInputConfig>,
) -> bool {
    if matches!(name, "file" | "directory") {
        return false;
    }
    let Some(variable) = snippet
        .variables
        .iter()
        .find(|variable| variable.name == name)
    else {
        return false;
    };
    if !matches!(variable.source, VariableSource::Free) {
        return false;
    }
    let constrained_locally = locals.get(name).is_some_and(variable_spec_constrains);
    let constrained_globally = globals.get(name).is_some_and(variable_input_constrains);
    !(constrained_locally || constrained_globally)
}

fn variable_spec_constrains(spec: &VariableSpec) -> bool {
    spec.default.is_some() || !spec.suggestions.is_empty() || spec.command.is_some()
}

fn variable_input_constrains(spec: &VariableInputConfig) -> bool {
    spec.default.is_some() || !spec.suggestions.is_empty() || spec.command.is_some()
}

fn command_sources(
    snippet: &Snippet,
    locals: &BTreeMap<String, VariableSpec>,
    globals: &BTreeMap<String, VariableInputConfig>,
) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for variable in &snippet.variables {
        if !seen.insert(variable.name.clone()) {
            continue;
        }
        match &variable.source {
            VariableSource::Command(command) => out.push((variable.name.clone(), command.clone())),
            VariableSource::Free => {
                if let Some(command) = locals
                    .get(&variable.name)
                    .and_then(|spec| spec.command.clone())
                    .or_else(|| {
                        globals
                            .get(&variable.name)
                            .and_then(|spec| spec.command.clone())
                    })
                {
                    out.push((variable.name.clone(), command));
                }
            }
            VariableSource::Default(_) => {}
        }
    }
    out
}
