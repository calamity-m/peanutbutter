//! Frontmatter and variable lint rules — broken frontmatter syntax, unused
//! file-local and config variables, and suggestion-source override conflicts.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use crate::config::VariableInputConfig;
use crate::domain::VariableSpec;

use super::{
    CODE_BROKEN_FRONTMATTER, CODE_FRONTMATTER_OVERRIDE, CODE_UNUSED_VARIABLE, FileContext,
    LintFinding, LintSeverity, finding,
};

/// Check for unterminated frontmatter blocks and malformed key/value lines.
pub(super) fn lint_frontmatter_source(path: &Path, content: &str) -> Vec<LintFinding> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.first().map(|l| l.trim()) != Some("---") {
        return Vec::new();
    }
    let Some(end) = lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, line)| line.trim() == "---")
        .map(|(idx, _)| idx)
    else {
        return vec![finding(
            LintSeverity::Error,
            CODE_BROKEN_FRONTMATTER,
            path.to_path_buf(),
            Some(1),
            None,
            "frontmatter block is not terminated".to_string(),
            None,
        )];
    };
    let mut out = Vec::new();
    let mut i = 1;
    while i < end {
        let trimmed = lines[i].trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }
        if !trimmed.contains(':') && !trimmed.starts_with('-') {
            out.push(finding(
                LintSeverity::Error,
                CODE_BROKEN_FRONTMATTER,
                path.to_path_buf(),
                Some(i + 1),
                None,
                "frontmatter line is not a supported key/value entry".to_string(),
                None,
            ));
        }
        if let Some((key, value)) = trimmed.split_once(':') {
            if key.trim().is_empty() {
                out.push(finding(
                    LintSeverity::Error,
                    CODE_BROKEN_FRONTMATTER,
                    path.to_path_buf(),
                    Some(i + 1),
                    None,
                    "frontmatter key is empty".to_string(),
                    None,
                ));
            }
            if value.trim().starts_with('[') && !value.trim().ends_with(']') {
                out.push(finding(
                    LintSeverity::Error,
                    CODE_BROKEN_FRONTMATTER,
                    path.to_path_buf(),
                    Some(i + 1),
                    None,
                    "inline list is not closed".to_string(),
                    None,
                ));
            }
        }
        i += 1;
    }
    out
}

/// Warn about file-level frontmatter variables not referenced by any snippet in the same file.
pub(super) fn lint_unused_file_variables(file: &FileContext) -> Vec<LintFinding> {
    let referenced = referenced_variables(file);
    let lines = frontmatter_variable_lines(&file.content);
    file.parsed
        .frontmatter
        .variables
        .keys()
        .filter(|name| !referenced.contains(*name))
        .map(|name| {
            finding(
                LintSeverity::Warning,
                CODE_UNUSED_VARIABLE,
                file.path.clone(),
                lines.get(name).copied(),
                None,
                format!(
                    "frontmatter variable '{name}' is not referenced by any snippet in this file"
                ),
                Some(
                    "remove the variable definition or add a matching <@name> placeholder"
                        .to_string(),
                ),
            )
        })
        .collect()
}

/// Warn about config-level variables not referenced by any snippet across all files.
pub(super) fn lint_unused_config_variables(
    globals: &BTreeMap<String, VariableInputConfig>,
    config_file: &Path,
    files: &[FileContext],
) -> Vec<LintFinding> {
    let mut referenced = HashSet::new();
    for file in files {
        referenced.extend(referenced_variables(file));
    }
    globals
        .keys()
        .filter(|name| !referenced.contains(*name))
        .map(|name| {
            finding(
                LintSeverity::Warning,
                CODE_UNUSED_VARIABLE,
                config_file.to_path_buf(),
                None,
                None,
                format!("config variable '{name}' is not referenced by any snippet"),
                Some(
                    "remove the variable definition or add a matching <@name> placeholder"
                        .to_string(),
                ),
            )
        })
        .collect()
}

/// Warn when a file-local variable overrides a global variable's suggestion source type.
pub(super) fn lint_frontmatter_overrides(
    file: &FileContext,
    globals: &BTreeMap<String, VariableInputConfig>,
) -> Vec<LintFinding> {
    let mut out = Vec::new();
    for (name, local) in &file.parsed.frontmatter.variables {
        let Some(global) = globals.get(name) else {
            continue;
        };
        let local_source = suggestion_source(local);
        let global_source = suggestion_source(global);
        if local_source != global_source && local_source != "none" && global_source != "none" {
            out.push(finding(
                LintSeverity::Warning,
                CODE_FRONTMATTER_OVERRIDE,
                file.path.clone(),
                Some(1),
                None,
                format!("file-local variable '{name}' overrides a global suggestion source"),
                Some("rename the local variable if it means something different".to_string()),
            ));
        }
    }
    out
}

fn referenced_variables(file: &FileContext) -> HashSet<String> {
    file.parsed
        .snippets
        .iter()
        .flat_map(|snippet| {
            snippet
                .variables
                .iter()
                .map(|variable| variable.name.clone())
        })
        .collect()
}

fn frontmatter_variable_lines(content: &str) -> HashMap<String, usize> {
    let lines: Vec<&str> = content.lines().collect();
    if lines.first().map(|line| line.trim()) != Some("---") {
        return HashMap::new();
    }
    let Some(end) = lines
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, line)| line.trim() == "---")
        .map(|(idx, _)| idx)
    else {
        return HashMap::new();
    };

    let mut out = HashMap::new();
    let Some(variables_line) = lines[..end]
        .iter()
        .enumerate()
        .skip(1)
        .find(|(_, line)| line.trim() == "variables:")
        .map(|(idx, _)| idx)
    else {
        return out;
    };

    for (idx, line) in lines.iter().enumerate().take(end).skip(variables_line + 1) {
        let trimmed = line.trim_start();
        let indent = line.len() - trimmed.len();
        if indent == 0 || trimmed.is_empty() || trimmed.starts_with('#') {
            break;
        }
        if let Some((name, rest)) = trimmed.split_once(':')
            && rest.trim().is_empty()
        {
            out.insert(name.trim().to_string(), idx + 1);
        }
    }
    out
}

fn suggestion_source(spec: &VariableSpec) -> &'static str {
    if spec.command.is_some() {
        "command"
    } else if !spec.suggestions.is_empty() {
        "suggestions"
    } else {
        "none"
    }
}
