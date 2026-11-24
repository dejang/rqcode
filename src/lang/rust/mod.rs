mod body;
mod extract;
mod macro_tokens;
mod normalize;
mod syntax;

use self::extract::RustExtractor;
use crate::ast::{
    CommentStyle, FileAst, FileRole, LanguageGrammar, LanguageId, LanguageSpec, ModulePath,
    SyntaxTree,
};
use crate::lang::{LanguageFrontend, common};
use ast_grep_core::Node;
use ast_grep_core::language::Language;
use ast_grep_core::tree_sitter::{LanguageExt, StrDoc, TSLanguage};
use ast_grep_language::Rust as AstGrepRust;
use std::borrow::Cow;
use std::path::Path;
use std::sync::Arc;

type TsNode<'a> = Node<'a, StrDoc<LanguageSpec>>;

struct RustGrammar;

impl LanguageGrammar for RustGrammar {
    fn id(&self) -> LanguageId {
        LanguageId::new("rust")
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["rust", "rs"]
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["rs"]
    }

    fn comment_style(&self) -> CommentStyle {
        CommentStyle::new("//", Some("//!"), Some(("/*", "*/")), &[])
    }

    fn member_separator(&self) -> &'static str {
        "::"
    }

    fn field_init_noun(&self) -> &'static str {
        "struct literal"
    }

    fn ts_language(&self) -> TSLanguage {
        AstGrepRust.get_ts_language()
    }

    fn kind_to_id(&self, kind: &str) -> u16 {
        AstGrepRust.kind_to_id(kind)
    }

    fn field_to_id(&self, field: &str) -> Option<u16> {
        AstGrepRust.field_to_id(field)
    }

    fn expando_char(&self) -> char {
        AstGrepRust.expando_char()
    }

    fn pre_process_pattern<'q>(&self, query: &'q str) -> Cow<'q, str> {
        AstGrepRust.pre_process_pattern(query)
    }
}

/// Construct the [`LanguageSpec`] for Rust.
#[must_use]
pub fn rust_language_spec() -> LanguageSpec {
    LanguageSpec::new(Arc::new(RustGrammar))
}

pub struct RustFrontend;

impl LanguageFrontend for RustFrontend {
    fn grammar(&self) -> LanguageSpec {
        rust_language_spec()
    }

    fn default_excludes(&self) -> &'static [&'static str] {
        &["target"]
    }

    fn module_path(&self, _root: &Path, relative_path: &Path) -> ModulePath {
        let components: Vec<String> = relative_path
            .components()
            .filter_map(|c| c.as_os_str().to_str().map(String::from))
            .collect();

        if components.is_empty() {
            return ModulePath::new(vec!["crate".to_string()]);
        }

        let mut segments = Vec::new();
        let start_idx = if components[0] == "src" {
            segments.push("crate".to_string());
            1
        } else {
            0
        };

        let last_idx = components.len() - 1;
        for (i, comp) in components.iter().enumerate().skip(start_idx) {
            if i == last_idx {
                let stem = comp.strip_suffix(".rs").unwrap_or(comp);
                if stem != "lib" && stem != "main" && stem != "mod" {
                    segments.push(stem.to_string());
                }
            } else {
                segments.push(comp.clone());
            }
        }

        if segments.is_empty() {
            segments.push("crate".to_string());
        }

        ModulePath::new(segments)
    }

    fn file_role(&self, relative_path: &Path) -> FileRole {
        common::classify_file_role(
            relative_path,
            &["tests", "benches", "examples"],
            &[],
            &["lib.rs"],
            &["main.rs"],
        )
    }

    fn extract_module(
        &self,
        tree: &SyntaxTree,
        module_path: ModulePath,
        role: FileRole,
    ) -> FileAst {
        RustExtractor::new(tree, module_path).extract_file(role)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::item::{ImportKind, ParamAccess, TypeRef};
    use crate::ast::similarity::{EligibilityConfig, SimilarityCorpus};

    fn extract_rust(rel_path: &str, src: &str) -> FileAst {
        let tree = SyntaxTree::parse(rel_path, rel_path, rust_language_spec(), src);
        RustFrontend.extract(&tree, Path::new("."))
    }

    #[test]
    fn infers_rust_module_paths_for_lib_mod_submodules_and_tests() {
        let f = RustFrontend;
        assert_eq!(
            f.module_path(Path::new("."), Path::new("src/lib.rs"))
                .to_string(),
            "crate"
        );
        assert_eq!(
            f.module_path(Path::new("."), Path::new("src/ast/mod.rs"))
                .to_string(),
            "crate::ast"
        );
        assert_eq!(
            f.module_path(Path::new("."), Path::new("src/ast/similarity/mod.rs"))
                .to_string(),
            "crate::ast::similarity"
        );
        assert_eq!(
            f.file_role(Path::new("tests/e2e.rs")),
            FileRole::IntegrationTest
        );
        assert_eq!(
            f.file_role(Path::new("src/main.rs")),
            FileRole::BinaryEntrypoint
        );
        assert_eq!(f.file_role(Path::new("src/lib.rs")), FileRole::LibraryRoot);
        assert_eq!(f.file_role(Path::new("src/order.rs")), FileRole::Module);
    }

    #[test]
    fn extracts_structs_impls_free_functions_field_writes_and_tests() {
        let src = r"
use crate::pricing::{apply_tax, Discount as Disc};

#[derive(Debug, Clone)]
pub struct Order {
    pub total: u64,
    id: u64,
}

impl Order {
    pub fn id(&self) -> u64 {
        self.id
    }
}

pub fn adjust(order: &mut Order, amount: u64) {
    order.total += amount;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checks_order() {
        let mut o = Order { total: 10, id: 1 };
        adjust(&mut o, 5);
    }
}
";
        let ast = extract_rust("src/order.rs", src);

        assert_eq!(ast.imports.len(), 2);
        assert_eq!(ast.imports[1].local_name(), "Disc");
        assert_eq!(ast.imports[1].symbol, "Discount");

        assert_eq!(ast.types.len(), 1);
        assert_eq!(ast.types[0].name, "Order");
        assert_eq!(ast.types[0].exposed_field_count(), 1);
        assert!(ast.types[0].has_annotation("Debug"));

        assert_eq!(ast.functions.len(), 3);

        let id_fn = &ast.functions[0];
        assert!(id_fn.is_inherent_method());
        assert_eq!(id_fn.owner_type.as_deref(), Some("Order"));

        let adjust_fn = &ast.functions[1];
        assert!(adjust_fn.is_free_function());
        assert_eq!(
            adjust_fn.params[0].type_name,
            Some(TypeRef::Named("Order".to_string()))
        );
        assert_eq!(adjust_fn.params[0].access, ParamAccess::MutableRef);
        assert_eq!(adjust_fn.field_accesses.len(), 1);
        assert!(adjust_fn.field_accesses[0].is_write);
        assert_eq!(adjust_fn.field_accesses[0].field_name, "total");

        let test_fn = &ast.functions[2];
        assert!(test_fn.is_test());
        assert!(test_fn.is_test_code());
        assert!(
            test_fn
                .constructed_types
                .contains(&TypeRef::Named("Order".to_string()))
        );
        assert!(test_fn.distinct_callees().contains(&"adjust"));
    }

    #[test]
    fn normalizes_renamed_variables_to_identical_similarity_score() {
        let src = r"
fn calc_a(price: u64, tax: u64) -> u64 {
    if price > 100 {
        return price + tax;
    }
    price
}

fn calc_b(cost: u64, fee: u64) -> u64 {
    if cost > 200 {
        return cost + fee;
    }
    cost
}
";
        let ast = extract_rust("src/math.rs", src);
        assert_eq!(ast.functions.len(), 2);
        let mut corpus = SimilarityCorpus::new(EligibilityConfig {
            min_nodes: 5,
            min_distinct_kinds: 2,
        });
        assert!(corpus.add_unit(0, &ast.functions[0].body_tree));
        assert!(corpus.add_unit(1, &ast.functions[1].body_tree));
        let matches = corpus.similar_pairs(0.99);
        assert_eq!(matches.len(), 1);
        assert!((matches[0].score() - 1.0).abs() < 1e-12);
    }

    #[test]
    fn extracts_pub_use_reexports_and_wildcard_imports() {
        let src = r"
pub use self::order::{Order, Item as LineItem};
pub use crate::legacy::*;
use crate::internal::*;
pub use external_crate::Logger as ExtLogger;
";
        let ast = extract_rust("src/models/mod.rs", src);

        assert_eq!(ast.imports.len(), 5);
        assert_eq!(ast.imports[0].kind, ImportKind::Named);
        assert_eq!(ast.imports[2].kind, ImportKind::Wildcard);
        assert_eq!(ast.imports[3].kind, ImportKind::Wildcard);

        assert_eq!(ast.exports.len(), 3);
        assert_eq!(ast.exports[0].symbol, "Order");
        assert_eq!(ast.exports[0].exported_name, "Order");
        assert_eq!(
            ast.exports[0].source_module,
            Some(ModulePath::new(vec![
                "crate".into(),
                "models".into(),
                "order".into()
            ]))
        );
        assert_eq!(ast.exports[1].symbol, "Item");
        assert_eq!(ast.exports[1].exported_name, "LineItem");
        assert!(ast.exports[2].is_wildcard);
        assert_eq!(
            ast.exports[2].source_module,
            Some(ModulePath::new(vec!["crate".into(), "legacy".into()]))
        );
    }
}
