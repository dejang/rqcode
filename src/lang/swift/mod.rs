mod body;
mod expr;
mod extract;
mod members;
mod normalize;
mod syntax;

use self::extract::SwiftExtractor;
use crate::ast::{
    CommentStyle, FileAst, FileRole, LanguageGrammar, LanguageId, LanguageSpec, ModulePath,
    SyntaxTree,
};
use crate::lang::{LanguageFrontend, common};
use ast_grep_core::Node;
use ast_grep_core::language::Language;
use ast_grep_core::tree_sitter::{LanguageExt, StrDoc, TSLanguage};
use ast_grep_language::Swift as AstGrepSwift;
use std::borrow::Cow;
use std::path::{Component, Path};
use std::sync::Arc;

type SwiftNode<'a> = Node<'a, StrDoc<LanguageSpec>>;

struct SwiftGrammar;

impl LanguageGrammar for SwiftGrammar {
    fn id(&self) -> LanguageId {
        LanguageId::new("swift")
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["swift"]
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["swift"]
    }

    fn comment_style(&self) -> CommentStyle {
        CommentStyle::new("//", None, Some(("/*", "*/")), &[])
    }

    fn member_separator(&self) -> &'static str {
        "."
    }

    fn ts_language(&self) -> TSLanguage {
        AstGrepSwift.get_ts_language()
    }

    fn kind_to_id(&self, kind: &str) -> u16 {
        AstGrepSwift.kind_to_id(kind)
    }

    fn field_to_id(&self, field: &str) -> Option<u16> {
        AstGrepSwift.field_to_id(field)
    }

    fn expando_char(&self) -> char {
        AstGrepSwift.expando_char()
    }

    fn pre_process_pattern<'q>(&self, query: &'q str) -> Cow<'q, str> {
        AstGrepSwift.pre_process_pattern(query)
    }
}

/// Construct the [`LanguageSpec`] for Swift.
#[must_use]
pub fn swift_language_spec() -> LanguageSpec {
    LanguageSpec::new(Arc::new(SwiftGrammar))
}

/// Injectable layout, test-marker, and standard container wrapper conventions for [`SwiftFrontend`].
#[derive(Debug, Clone)]
pub struct SwiftConventions {
    pub source_roots: Vec<String>,
    pub test_roots: Vec<String>,
    pub integration_test_dirs: Vec<String>,
    pub integration_test_dir_suffixes: Vec<String>,
    pub unit_test_target_suffixes: Vec<String>,
    pub unit_test_file_suffixes: Vec<String>,
    pub test_stem_suffixes: Vec<String>,
    pub entrypoint_files: Vec<String>,
    pub test_base_classes: Vec<String>,
    pub test_suite_attributes: Vec<String>,
    pub test_method_attributes: Vec<String>,
    pub test_container_name_suffixes: Vec<String>,
    pub xctest_method_prefix: String,
    pub transparent_type_wrappers: Vec<String>,
}

impl Default for SwiftConventions {
    fn default() -> Self {
        let to_vec = |items: &[&str]| items.iter().map(|s| (*s).to_string()).collect();
        Self {
            source_roots: to_vec(&["Sources"]),
            test_roots: to_vec(&["Tests"]),
            integration_test_dirs: to_vec(&[
                "tests",
                "e2e",
                "IntegrationTests",
                "UITests",
                "E2ETests",
            ]),
            integration_test_dir_suffixes: to_vec(&["UITests", "IntegrationTests", "E2ETests"]),
            unit_test_target_suffixes: to_vec(&["UnitTests", "Tests"]),
            unit_test_file_suffixes: to_vec(&["Tests.swift"]),
            test_stem_suffixes: to_vec(&["Tests", "Test"]),
            entrypoint_files: to_vec(&["main.swift"]),
            test_base_classes: to_vec(&["XCTestCase"]),
            test_suite_attributes: to_vec(&["Suite"]),
            test_method_attributes: to_vec(&["Test"]),
            test_container_name_suffixes: to_vec(&["Tests"]),
            xctest_method_prefix: String::from("test"),
            // Swift standard library container wrappers only (no UI/app framework names).
            transparent_type_wrappers: to_vec(&[
                "Optional",
                "Array",
                "ContiguousArray",
                "ArraySlice",
                "Set",
                "Result",
                "Task",
                "AsyncStream",
                "AsyncThrowingStream",
            ]),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SwiftFrontend {
    conventions: SwiftConventions,
}

impl SwiftFrontend {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_conventions(conventions: SwiftConventions) -> Self {
        Self { conventions }
    }

    fn is_source_root(&self, dir: &str) -> bool {
        self.conventions.source_roots.iter().any(|r| r == dir)
    }

    fn is_test_root(&self, dir: &str) -> bool {
        self.conventions.test_roots.iter().any(|r| r == dir)
    }

    fn is_entrypoint_file(&self, file_name: &str) -> bool {
        self.conventions
            .entrypoint_files
            .iter()
            .any(|f| f == file_name)
    }

    fn is_integration_test_dir(&self, dir: &str) -> bool {
        self.conventions
            .integration_test_dirs
            .iter()
            .any(|d| d == dir)
            || self
                .conventions
                .integration_test_dir_suffixes
                .iter()
                .any(|suffix| dir.ends_with(suffix))
    }

    fn strip_test_target_suffix<'a>(&self, dir: &'a str) -> Option<&'a str> {
        if self.is_integration_test_dir(dir) || self.is_test_root(dir) {
            return None;
        }
        for suffix in &self.conventions.unit_test_target_suffixes {
            if let Some(stem) = dir.strip_suffix(suffix.as_str())
                && !stem.is_empty()
            {
                return Some(stem);
            }
        }
        None
    }

    fn strip_test_stem<'a>(&self, stem: &'a str) -> &'a str {
        for suffix in &self.conventions.test_stem_suffixes {
            if let Some(stripped) = stem.strip_suffix(suffix.as_str())
                && !stripped.is_empty()
            {
                return stripped;
            }
        }
        stem
    }

    fn resolve_root_prefix(&self, components: &[&str], segments: &mut Vec<String>) -> usize {
        let dirs = &components[..components.len().saturating_sub(1)];
        let source_idx = dirs.iter().position(|&d| self.is_source_root(d));
        let test_idx = dirs.iter().position(|&d| self.is_test_root(d));

        if let Some(s_idx) = source_idx
            && test_idx.is_none_or(|t_idx| s_idx < t_idx)
        {
            return s_idx + 1;
        }
        if let Some(t_idx) = test_idx
            && let Some(&target_dir) = dirs.get(t_idx + 1)
            && let Some(target) = self.strip_test_target_suffix(target_dir)
        {
            segments.push(target.to_string());
            return t_idx + 2;
        }
        if let Some((idx, target)) = dirs
            .iter()
            .enumerate()
            .find_map(|(i, &d)| self.strip_test_target_suffix(d).map(|t| (i, t)))
        {
            segments.push(target.to_string());
            return idx + 1;
        }
        0
    }
}

impl LanguageFrontend for SwiftFrontend {
    fn grammar(&self) -> LanguageSpec {
        swift_language_spec()
    }

    fn default_excludes(&self) -> &'static [&'static str] {
        &[".build", ".swiftpm", "DerivedData", "Pods", "Carthage"]
    }

    fn module_path(&self, _root: &Path, relative_path: &Path) -> ModulePath {
        let components = normal_components(relative_path);
        if components.is_empty() {
            return ModulePath::new(vec![String::from("main")]);
        }

        let is_test = self.file_role(relative_path).is_test_file();
        let mut segments = Vec::with_capacity(components.len());
        let start_idx = self.resolve_root_prefix(&components, &mut segments);
        let last_idx = components.len() - 1;
        let grammar = self.grammar();

        for (idx, &comp) in components.iter().enumerate().skip(start_idx) {
            if idx == last_idx {
                let without_ext = grammar
                    .extensions()
                    .iter()
                    .find_map(|ext| comp.strip_suffix(&format!(".{ext}")))
                    .unwrap_or(comp);
                let stem = if is_test {
                    self.strip_test_stem(without_ext)
                } else {
                    without_ext
                };
                let duplicates_parent = segments.last().is_some_and(|parent| parent == stem);
                if !stem.is_empty() && !duplicates_parent {
                    segments.push(stem.to_string());
                }
            } else {
                segments.push(comp.to_string());
            }
        }

        if segments.is_empty() {
            segments.push(String::from("main"));
        }

        ModulePath::new(segments)
    }

    fn file_role(&self, relative_path: &Path) -> FileRole {
        let components = normal_components(relative_path);
        let Some(&file_name) = components.last() else {
            return FileRole::Module;
        };
        let dirs = &components[..components.len() - 1];

        // Production code under `Sources/` (root or nested package `Packages/*/Sources/`) is
        // never classified as a test even if a file is named `ProductSpec.swift` or `LoadTest.swift`.
        let earliest_root = dirs.iter().find(|&&d| {
            self.is_source_root(d) || self.is_test_root(d) || self.is_integration_test_dir(d)
        });
        if earliest_root.is_some_and(|&d| self.is_source_root(d)) {
            return if self.is_entrypoint_file(file_name) {
                FileRole::BinaryEntrypoint
            } else {
                FileRole::Module
            };
        }

        if dirs.iter().any(|&d| self.is_integration_test_dir(d)) {
            return FileRole::IntegrationTest;
        }
        if let Some(t_idx) = dirs.iter().position(|&d| self.is_test_root(d)) {
            return if dirs
                .get(t_idx + 1)
                .and_then(|&target| self.strip_test_target_suffix(target))
                .is_some()
            {
                FileRole::UnitTest
            } else {
                FileRole::IntegrationTest
            };
        }
        if dirs
            .iter()
            .any(|&target| self.strip_test_target_suffix(target).is_some())
        {
            return FileRole::UnitTest;
        }

        let integration_dirs: Vec<&str> = self
            .conventions
            .integration_test_dirs
            .iter()
            .map(String::as_str)
            .collect();
        let unit_suffixes: Vec<&str> = self
            .conventions
            .unit_test_file_suffixes
            .iter()
            .map(String::as_str)
            .collect();
        let entrypoints: Vec<&str> = self
            .conventions
            .entrypoint_files
            .iter()
            .map(String::as_str)
            .collect();
        common::classify_file_role(
            relative_path,
            &integration_dirs,
            &unit_suffixes,
            &[],
            &entrypoints,
        )
    }

    fn extract_module(
        &self,
        tree: &SyntaxTree,
        module_path: ModulePath,
        role: FileRole,
    ) -> FileAst {
        SwiftExtractor::new(tree, module_path, role, &self.conventions).extract_file()
    }
}

fn normal_components(path: &Path) -> Vec<&str> {
    path.components()
        .filter_map(|c| match c {
            Component::Normal(seg) => seg.to_str(),
            _ => None,
        })
        .collect()
}
