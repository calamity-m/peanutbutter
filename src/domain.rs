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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Frontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VariableSource {
    Free,
    Command(String),
    Default(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Variable {
    pub name: String,
    pub source: VariableSource,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snippet {
    pub id: SnippetId,
    pub name: String,
    pub description: String,
    pub body: String,
    pub variables: Vec<Variable>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnippetFile {
    pub path: PathBuf,
    pub relative_path: PathBuf,
    pub frontmatter: Frontmatter,
    pub snippets: Vec<Snippet>,
}
