pub(crate) mod common;
pub mod rust;
pub mod swift;
pub mod typescript;

use crate::ast::{FileAst, FileRole, LanguageId, LanguageSpec, ModulePath, SyntaxTree};
use std::collections::HashSet;
use std::path::Path;

/// Translates a language-specific Tree-sitter CST into the language-agnostic `FileAst`.
pub trait LanguageFrontend: Send + Sync {
    /// Returns this frontend's default dialect grammar. Path-aware callers should use
    /// [`Self::grammar_for_path`] so multi-dialect frontends can select the matching grammar.
    fn grammar(&self) -> LanguageSpec;

    /// Returns the Tree-sitter grammar for a specific source file path.
    /// Multi-dialect frontends (such as TypeScript + TSX) override this to select
    /// the matching dialect grammar instead of the default [`Self::grammar`].
    fn grammar_for_path(&self, _path: &Path) -> LanguageSpec {
        self.grammar()
    }

    /// Additional dialect grammars owned by this frontend (for `ast-grep`
    /// rule resolution and ID lookup).
    fn additional_grammars(&self) -> Vec<LanguageSpec> {
        Vec::new()
    }

    fn id(&self) -> LanguageId {
        self.grammar().id()
    }

    fn extensions(&self) -> &'static [&'static str] {
        self.grammar().extensions()
    }

    fn default_excludes(&self) -> &'static [&'static str] {
        &[]
    }

    fn module_path(&self, root: &Path, relative_path: &Path) -> ModulePath;
    fn file_role(&self, relative_path: &Path) -> FileRole;
    fn extract_module(&self, tree: &SyntaxTree, module_path: ModulePath, role: FileRole)
    -> FileAst;

    fn extract(&self, tree: &SyntaxTree, root: &Path) -> FileAst {
        let module_path = self.module_path(root, tree.relative_path());
        let role = self.file_role(tree.relative_path());
        self.extract_module(tree, module_path, role)
    }
}

/// Registry of active language frontends, allowing polyglot expansion without touching rules.
pub struct LanguageRegistry {
    frontends: Vec<Box<dyn LanguageFrontend>>,
}

impl LanguageRegistry {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            frontends: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_defaults() -> Self {
        let mut registry = Self::empty();
        registry.register(Box::new(rust::RustFrontend));
        registry.register(Box::new(typescript::TypeScriptFrontend::new()));
        registry.register(Box::new(swift::SwiftFrontend::new()));
        registry
    }

    pub fn register(&mut self, frontend: Box<dyn LanguageFrontend>) {
        self.frontends.push(frontend);
    }

    #[must_use]
    pub fn frontend_for_path(&self, path: &Path) -> Option<&dyn LanguageFrontend> {
        let ext = path.extension()?.to_str()?;
        self.frontends
            .iter()
            .find(|f| f.extensions().iter().any(|e| e.eq_ignore_ascii_case(ext)))
            .map(Box::as_ref)
    }

    #[must_use]
    pub fn frontend_for_id(&self, id: LanguageId) -> Option<&dyn LanguageFrontend> {
        self.frontends
            .iter()
            .find(|f| f.id() == id || f.additional_grammars().iter().any(|g| g.id() == id))
            .map(Box::as_ref)
    }

    #[must_use]
    pub fn resolve_language(&self, name: &str) -> Option<LanguageSpec> {
        for frontend in &self.frontends {
            let primary = frontend.grammar();
            if primary.matches_name(name) {
                return Some(primary);
            }
            for extra in frontend.additional_grammars() {
                if extra.matches_name(name) {
                    return Some(extra);
                }
            }
        }
        None
    }

    #[must_use]
    pub fn default_excludes(&self) -> Vec<&'static str> {
        let mut seen = HashSet::new();
        let mut excludes = Vec::new();
        for frontend in &self.frontends {
            for &exclude in frontend.default_excludes() {
                if seen.insert(exclude) {
                    excludes.push(exclude);
                }
            }
        }
        excludes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_resolves_builtin_languages_and_ignores_unknown_extensions() {
        let registry = LanguageRegistry::with_defaults();
        let rs = registry.frontend_for_path(Path::new("src/main.rs"));
        assert_eq!(rs.map(LanguageFrontend::id), Some(LanguageId::new("rust")));
        let ts = registry.frontend_for_path(Path::new("src/order.ts"));
        assert_eq!(
            ts.map(LanguageFrontend::id),
            Some(LanguageId::new("typescript"))
        );
        let sw = registry.frontend_for_path(Path::new("Sources/Shop/Order.swift"));
        assert_eq!(sw.map(LanguageFrontend::id), Some(LanguageId::new("swift")));
        assert!(registry.frontend_for_path(Path::new("README.md")).is_none());
        assert_eq!(
            registry.resolve_language("Rust").map(|s| s.id()),
            Some(LanguageId::new("rust"))
        );
        assert_eq!(
            registry.resolve_language("rs").map(|s| s.id()),
            Some(LanguageId::new("rust"))
        );
        assert_eq!(
            registry.resolve_language("TypeScript").map(|s| s.id()),
            Some(LanguageId::new("typescript"))
        );
        assert_eq!(
            registry.resolve_language("ts").map(|s| s.id()),
            Some(LanguageId::new("typescript"))
        );
        assert_eq!(
            registry.resolve_language("Swift").map(|s| s.id()),
            Some(LanguageId::new("swift"))
        );
        assert_eq!(
            registry.resolve_language("swift").map(|s| s.id()),
            Some(LanguageId::new("swift"))
        );
        assert!(registry.resolve_language("cobol").is_none());
        assert_eq!(
            registry.default_excludes(),
            vec![
                "target",
                "node_modules",
                "dist",
                "build",
                ".next",
                "coverage",
                ".build",
                ".swiftpm",
                "DerivedData",
                "Pods",
                "Carthage",
            ]
        );
    }
}
