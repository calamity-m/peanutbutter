use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct SnippetId(String);

impl SnippetId {
    /// Constructs an id of the form `"relative/path.md#heading-slug"`.
    /// The `#` separator is the only `#` in the string; `frecency.rs`
    /// relies on `split_once('#')` to round-trip it. Neither `relative_path`
    /// nor `heading_slug` may contain a literal `#`.
    pub fn new(relative_path: &str, heading_slug: &str) -> Self {
        debug_assert!(!relative_path.contains('#'), "path must not contain '#'");
        debug_assert!(!heading_slug.contains('#'), "slug must not contain '#'");
        Self(format!("{relative_path}#{heading_slug}"))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SnippetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// Optional YAML frontmatter from the top of a snippet file.
///
/// All fields are optional; absent frontmatter produces the `Default` value.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    /// Human-readable file title (the `name:` key).
    pub name: Option<String>,
    /// Short prose description of the file's contents (the `description:` key).
    pub description: Option<String>,
    /// Searchable tags (`tags: [a, b]` or block-list form).
    pub tags: Vec<String>,
}

/// How a template variable `<@name[:source]>` should be filled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariableSource {
    /// No inline source; suggestions come from config overrides or builtins
    /// (`file`, `directory`).
    Free,
    /// Run `bash -c <command>` and split stdout into suggestion lines.
    Command(String),
    /// Pre-populated default value (`<@name:?default>`); can be overridden.
    Default(String),
}

/// A single `<@name[:source]>` placeholder parsed from a snippet body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    /// Identifier used to match the placeholder and store the filled value.
    pub name: String,
    /// Where to obtain suggestions or a default for this variable.
    pub source: VariableSource,
}

/// A single executable snippet: a named `##` section with a fenced code block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    /// Stable identifier derived from the file's relative path and heading slug.
    pub id: SnippetId,
    /// The `##` heading text, used as the display name.
    pub name: String,
    /// Prose between the heading and the opening code fence (may be empty).
    pub description: String,
    /// The raw body of the first fenced code block under this heading.
    pub body: String,
    /// Variables extracted from `body` in order of first appearance.
    pub variables: Vec<Variable>,
}

/// A parsed Markdown file: its resolved paths, optional frontmatter, and all
/// snippets extracted from `##` sections.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetFile {
    /// Absolute path to the source file on disk.
    pub path: PathBuf,
    /// Path relative to the snippet root used to build [`SnippetId`]s.
    pub relative_path: PathBuf,
    /// File-level metadata from the optional `---` YAML block.
    pub frontmatter: Frontmatter,
    /// All snippets parsed from this file, in document order.
    pub snippets: Vec<Snippet>,
}
