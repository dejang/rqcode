pub(crate) mod common;
pub mod rust;
pub mod typescript;

use crate::ast::{FileAst, FileRole, LanguageId, LanguageSpec, ModulePath, SyntaxTree};
use std::collections::HashSet;
use std::path::Path;

/// Translates a language-specific Tree-sitter CST into the language-agnostic `FileAst`.
pub trait LanguageFrontend: Send + Sync {
    fn grammar(&self) -> LanguageSpec;

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
        registry.register(Box::new(typescript::TypeScriptFrontend));
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
            .find(|f| f.id() == id)
            .map(Box::as_ref)
    }

    #[must_use]
    pub fn resolve_language(&self, name: &str) -> Option<LanguageSpec> {
        self.frontends
            .iter()
            .map(|f| f.grammar())
            .find(|spec| spec.matches_name(name))
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
    fn default_registry_resolves_rust_files_and_ignores_unknown_extensions() {
        let registry = LanguageRegistry::with_defaults();
        let rs = registry.frontend_for_path(Path::new("src/main.rs"));
        assert_eq!(rs.map(LanguageFrontend::id), Some(LanguageId::new("rust")));
        let ts = registry.frontend_for_path(Path::new("src/order.ts"));
        assert_eq!(
            ts.map(LanguageFrontend::id),
            Some(LanguageId::new("typescript"))
        );
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
        assert!(registry.resolve_language("cobol").is_none());
        assert_eq!(
            registry.default_excludes(),
            vec![
                "target",
                "node_modules",
                "dist",
                "build",
                ".next",
                "coverage"
            ]
        );
    }
}
