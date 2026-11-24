pub mod codebase;
pub mod item;
pub mod language;
pub mod similarity;

pub use self::language::{CommentStyle, LanguageGrammar, LanguageId, LanguageSpec};

use crate::ast::item::{ExportDecl, FunctionDecl, ImportDecl, Suppression, TypeDecl};
use crate::diagnostic::Span;
use ast_grep_core::matcher::Pattern;
use ast_grep_core::tree_sitter::{LanguageExt, StrDoc};
use ast_grep_core::{AstGrep, Node};
use std::fmt;
use std::path::{Path, PathBuf};
use thiserror::Error;

/// Logical module/namespace path (e.g., `crate::ast::similarity` or `src::models::order`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ModulePath(Vec<String>);

impl ModulePath {
    #[must_use]
    pub fn new(segments: Vec<String>) -> Self {
        Self(segments)
    }

    #[must_use]
    pub fn segments(&self) -> &[String] {
        &self.0
    }

    #[must_use]
    pub fn push(&self, segment: impl Into<String>) -> Self {
        let mut next = self.0.clone();
        next.push(segment.into());
        Self(next)
    }

    #[must_use]
    pub fn is_same_or_child_of(&self, ancestor: &Self) -> bool {
        self.0.len() >= ancestor.0.len() && self.0[..ancestor.0.len()] == ancestor.0[..]
    }

    /// Check whether `self` ends with the given reversed segment suffix.
    #[must_use]
    pub fn matches_suffix<'a>(&self, prefix_rev: impl Iterator<Item = &'a str>) -> bool {
        let mut segs_rev = self.0.iter().rev().map(String::as_str);
        for expected in prefix_rev {
            if segs_rev.next() != Some(expected) {
                return false;
            }
        }
        true
    }

    /// Check whether a canonical qualified path refers to a non-empty suffix of `self`.
    #[must_use]
    pub fn matches_qualified_path(&self, raw_path: &str) -> bool {
        let mut parts = raw_path.rsplit("::").filter(|s| !s.is_empty()).peekable();
        parts.peek().is_some() && self.matches_suffix(parts)
    }
}

impl fmt::Display for ModulePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0.join("::"))
    }
}

/// Borrowed view of a canonical qualified IR path (`prefix::member`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QualifiedPath<'a> {
    prefix: &'a str,
    member: &'a str,
}

impl<'a> QualifiedPath<'a> {
    /// Split a canonical qualified IR path into its module prefix and trailing member name.
    #[must_use]
    pub fn parse(raw: &'a str) -> Option<Self> {
        let trimmed = raw.trim_end_matches("::");
        let (prefix, member) = trimmed.rsplit_once("::")?;
        if member.is_empty() {
            return None;
        }
        Some(Self { prefix, member })
    }

    #[must_use]
    pub const fn prefix(self) -> &'a str {
        self.prefix
    }

    #[must_use]
    pub const fn member(self) -> &'a str {
        self.member
    }

    #[must_use]
    pub fn matches_module(self, module: &ModulePath) -> bool {
        module.matches_suffix(self.prefix.rsplit("::").filter(|s| !s.is_empty()))
    }
}

/// Language-agnostic role of a source file within a project/package.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum FileRole {
    /// Regular production module.
    #[default]
    Module,
    /// Executable entrypoint file (e.g., Rust `src/main.rs`, TS `src/cli.ts`).
    BinaryEntrypoint,
    /// Library/package root file (e.g., Rust `src/lib.rs`, TS `src/index.ts`).
    LibraryRoot,
    /// Dedicated unit test file co-located with a production module (e.g., `order.test.ts`).
    UnitTest,
    /// Standalone integration test, benchmark, or example file (e.g., `tests/e2e.rs`).
    IntegrationTest,
}

impl FileRole {
    #[must_use]
    pub const fn is_integration_test(self) -> bool {
        matches!(self, Self::IntegrationTest)
    }

    #[must_use]
    pub const fn is_test_file(self) -> bool {
        matches!(self, Self::UnitTest | Self::IntegrationTest)
    }
}

#[derive(Debug, Error)]
pub enum SyntaxError {
    #[error("invalid ast-grep pattern `{pattern}` for {language}: {message}")]
    InvalidPattern {
        language: LanguageId,
        pattern: String,
        message: String,
    },
}

/// A structural match returned by `SyntaxTree::find_pattern`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatternMatch {
    span: Span,
    text: String,
}

impl PatternMatch {
    #[must_use]
    pub fn span(&self) -> &Span {
        &self.span
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }
}

/// Single-pass Tree-sitter / `ast-grep` Concrete Syntax Tree for one source file.
pub struct SyntaxTree {
    path: PathBuf,
    relative_path: PathBuf,
    language: LanguageSpec,
    source: String,
    grep: AstGrep<StrDoc<LanguageSpec>>,
}

impl SyntaxTree {
    pub fn parse(
        path: impl Into<PathBuf>,
        relative_path: impl Into<PathBuf>,
        language: LanguageSpec,
        source: impl Into<String>,
    ) -> Self {
        let source = source.into();
        let grep = language.ast_grep(&source);
        Self {
            path: path.into(),
            relative_path: relative_path.into(),
            language,
            source,
            grep,
        }
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[must_use]
    pub fn relative_path(&self) -> &Path {
        &self.relative_path
    }

    #[must_use]
    pub fn language(&self) -> &LanguageSpec {
        &self.language
    }

    #[must_use]
    pub fn language_id(&self) -> LanguageId {
        self.language.id()
    }

    #[must_use]
    pub fn source(&self) -> &str {
        &self.source
    }

    #[must_use]
    pub fn ast_grep(&self) -> &AstGrep<StrDoc<LanguageSpec>> {
        &self.grep
    }

    #[must_use]
    pub fn root(&self) -> Node<'_, StrDoc<LanguageSpec>> {
        self.grep.root()
    }

    #[must_use]
    pub fn node_span(&self, node: &Node<'_, StrDoc<LanguageSpec>>) -> Span {
        self.nodes_span(node, node)
    }

    #[must_use]
    pub fn nodes_span(
        &self,
        start_node: &Node<'_, StrDoc<LanguageSpec>>,
        end_node: &Node<'_, StrDoc<LanguageSpec>>,
    ) -> Span {
        let start = start_node.start_pos();
        let end = end_node.end_pos();
        Span::new(
            self.relative_path.clone(),
            start.line() + 1,
            start.column(start_node) + 1,
            end.line() + 1,
            end.column(end_node) + 1,
        )
    }

    /// Find all nodes matching an `ast-grep` pattern string.
    ///
    /// # Errors
    /// Returns [`SyntaxError::InvalidPattern`] if `pattern_src` cannot be compiled for this tree's language.
    pub fn find_pattern(&self, pattern_src: &str) -> Result<Vec<PatternMatch>, SyntaxError> {
        let pattern = Pattern::try_new(pattern_src, self.language.clone()).map_err(|err| {
            SyntaxError::InvalidPattern {
                language: self.language.id(),
                pattern: pattern_src.to_string(),
                message: err.to_string(),
            }
        })?;

        let matches = self
            .root()
            .find_all(&pattern)
            .map(|m| {
                let node = m.get_node();
                PatternMatch {
                    span: self.node_span(node),
                    text: node.text().into_owned(),
                }
            })
            .collect();

        Ok(matches)
    }

    /// Extract inline `rqcode:allow(rule-id)` and `rqcode:allow-file(rule-id)` directives from comments.
    #[must_use]
    pub fn extract_suppressions(&self) -> Vec<Suppression> {
        self.language
            .comment_style()
            .extract_suppressions(&self.source)
    }
}

/// Language-agnostic semantic representation of a single parsed source file.
#[derive(Debug, Clone)]
pub struct FileAst {
    pub path: PathBuf,
    pub relative_path: PathBuf,
    pub module_path: ModulePath,
    pub language: LanguageId,
    pub member_separator: &'static str,
    pub role: FileRole,
    pub imports: Vec<ImportDecl>,
    pub exports: Vec<ExportDecl>,
    pub types: Vec<TypeDecl>,
    pub functions: Vec<FunctionDecl>,
    pub top_level_idents: Vec<String>,
    pub suppressions: Vec<Suppression>,
}

impl FileAst {
    #[must_use]
    pub const fn is_integration_test_file(&self) -> bool {
        self.role.is_integration_test()
    }

    #[must_use]
    pub const fn is_test_file(&self) -> bool {
        self.role.is_test_file()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::rust::rust_language_spec;

    #[test]
    fn syntax_tree_matches_rust_ast_grep_pattern() {
        let src = "fn demo() {\n    todo!(\"later\");\n}\n";
        let tree = SyntaxTree::parse("src/demo.rs", "src/demo.rs", rust_language_spec(), src);
        let matches = tree
            .find_pattern("todo!($$$ARGS)")
            .expect("valid rust pattern");

        assert_eq!(matches.len(), 1);
        assert_eq!(matches[0].span().start_line(), 2);
        assert_eq!(matches[0].text(), "todo!(\"later\")");
    }

    #[test]
    fn extracts_line_and_file_level_suppressions() {
        let src = r"//! rqcode:allow(scattered-tests)
// rqcode:allow(duplicate-function, no-dbg-macro)
fn example() {
    #[allow(unused)]
    let x = 1;
}
";
        let tree = SyntaxTree::parse("src/ex.rs", "src/ex.rs", rust_language_spec(), src);
        let supps = tree.extract_suppressions();

        assert_eq!(supps.len(), 3);
        assert!(supps.iter().any(|s| s.matches("scattered-tests", 50)));
        assert!(supps.iter().any(|s| s.matches("duplicate-function", 3)));
        assert!(supps.iter().any(|s| s.matches("no-dbg-macro", 3)));
    }
}
