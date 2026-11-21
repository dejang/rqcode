mod bind;
mod model;
mod resolve;

pub use self::model::{
    ExternalFuncUsage, FileId, FuncId, IndexedFunc, IndexedType, ResolvedFieldInit, ResolvedParam,
    TypeId, TypeTestProfile,
};

use self::bind::FunctionBinder;
use crate::ast::item::{ImportKind, ParamAccess};
use crate::ast::{FileAst, FileRole, SyntaxTree};
use crate::diagnostic::Span;
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Language-agnostic resolved codebase graph combining semantic AST facts and raw CSTs.
pub struct Codebase {
    trees: Vec<SyntaxTree>,
    files: Vec<FileAst>,
    types: Vec<IndexedType>,
    functions: Vec<IndexedFunc>,
    file_by_rel_path: HashMap<std::path::PathBuf, FileId>,
    free_funcs_by_name: HashMap<String, Vec<FuncId>>,
    func_param_types: Vec<HashSet<TypeId>>,
    file_imports_by_local_name: Vec<HashSet<String>>,
    is_binary_only_package: bool,
}

impl Codebase {
    #[must_use]
    pub fn build(trees: Vec<SyntaxTree>, files: Vec<FileAst>) -> Self {
        let mut file_by_rel_path = HashMap::with_capacity(files.len());
        let mut types = Vec::new();
        let mut has_binary_entry = false;
        let mut has_lib_root = false;

        for (file_idx, file) in files.iter().enumerate() {
            let file_id = FileId(file_idx);
            file_by_rel_path.insert(file.relative_path.clone(), file_id);
            match file.role {
                FileRole::BinaryEntrypoint => has_binary_entry = true,
                FileRole::LibraryRoot => has_lib_root = true,
                FileRole::Module | FileRole::UnitTest | FileRole::IntegrationTest => {}
            }

            for decl in &file.types {
                let id = TypeId(types.len());
                types.push(IndexedType {
                    id,
                    file_id,
                    module_path: file.module_path.clone(),
                    decl: decl.clone(),
                });
            }
        }

        let binder = FunctionBinder::new(&files, &types);
        let mut functions = Vec::new();
        let mut free_funcs_by_name: HashMap<String, Vec<FuncId>> = HashMap::new();

        for (file_idx, file) in files.iter().enumerate() {
            let file_id = FileId(file_idx);
            for decl in &file.functions {
                let id = FuncId(functions.len());
                if decl.is_free_function() && !decl.is_test_code() && !file.is_test_file() {
                    free_funcs_by_name
                        .entry(decl.name.clone())
                        .or_default()
                        .push(id);
                }
                functions.push(binder.bind_func(id, file_id, file, decl));
            }
        }
        binder.finalize_functions(&mut functions);
        drop(binder);

        let func_param_types = functions
            .iter()
            .map(|f| {
                f.resolved_params
                    .iter()
                    .filter_map(|p| p.resolved_type)
                    .collect()
            })
            .collect();

        let file_imports_by_local_name = files
            .iter()
            .map(|f| {
                f.imports
                    .iter()
                    .filter(|imp| imp.kind != ImportKind::Wildcard)
                    .map(|imp| imp.local_name().to_string())
                    .collect()
            })
            .collect();

        Self {
            trees,
            files,
            types,
            functions,
            file_by_rel_path,
            free_funcs_by_name,
            func_param_types,
            file_imports_by_local_name,
            is_binary_only_package: has_binary_entry && !has_lib_root,
        }
    }

    #[must_use]
    pub fn trees(&self) -> &[SyntaxTree] {
        &self.trees
    }

    #[must_use]
    pub fn files(&self) -> &[FileAst] {
        &self.files
    }

    #[must_use]
    pub fn file(&self, id: FileId) -> &FileAst {
        &self.files[id.0]
    }

    #[must_use]
    pub fn field_init_noun(&self, id: FileId) -> &'static str {
        self.trees.get(id.0).map_or("field initializer", |tree| {
            tree.language().field_init_noun()
        })
    }

    #[must_use]
    pub fn types(&self) -> &[IndexedType] {
        &self.types
    }

    #[must_use]
    pub fn ty(&self, id: TypeId) -> &IndexedType {
        &self.types[id.0]
    }

    #[must_use]
    pub fn functions(&self) -> &[IndexedFunc] {
        &self.functions
    }

    #[must_use]
    pub fn func(&self, id: FuncId) -> &IndexedFunc {
        &self.functions[id.0]
    }

    #[must_use]
    pub fn is_binary_only_package(&self) -> bool {
        self.is_binary_only_package
    }

    #[must_use]
    pub fn file_imports(&self, file_id: FileId) -> &HashSet<String> {
        &self.file_imports_by_local_name[file_id.0]
    }

    #[must_use]
    pub fn inherent_methods_for(&self, type_id: TypeId) -> Vec<&IndexedFunc> {
        self.functions
            .iter()
            .filter(|f| f.decl.is_inherent_method() && f.resolved_owner == Some(type_id))
            .collect()
    }

    #[must_use]
    pub fn split_impl_methods_for(&self, type_id: TypeId) -> Vec<&IndexedFunc> {
        let ty = &self.types[type_id.0];
        self.inherent_methods_for(type_id)
            .into_iter()
            .filter(|f| f.module_path != ty.module_path)
            .collect()
    }

    /// Find all non-test free functions defined outside `type_id`'s module that operate on it.
    #[must_use]
    pub fn external_free_functions_for(&self, type_id: TypeId) -> Vec<ExternalFuncUsage<'_>> {
        let ty = &self.types[type_id.0];
        let mut usages = Vec::new();
        let mut param_names: HashSet<&str> = HashSet::new();
        let mut other_project_types: HashSet<TypeId> = HashSet::new();
        let mut reads_seen: HashSet<&str> = HashSet::new();
        let mut writes_seen: HashSet<&str> = HashSet::new();
        let mut reads = Vec::new();
        let mut writes = Vec::new();

        for func in &self.functions {
            if !func.decl.is_free_function()
                || func.decl.is_test_code()
                || self.files[func.file_id.0].is_test_file()
                || func.module_path == ty.module_path
            {
                continue;
            }

            param_names.clear();
            other_project_types.clear();
            let mut takes_mut_ref = false;

            for param in &func.resolved_params {
                match param.resolved_type {
                    Some(tid) if tid == type_id => {
                        param_names.insert(param.decl.name.as_str());
                        if param.decl.access == ParamAccess::MutableRef {
                            takes_mut_ref = true;
                        }
                    }
                    Some(tid) => {
                        other_project_types.insert(tid);
                    }
                    None => {}
                }
            }

            if param_names.is_empty() {
                continue;
            }

            reads_seen.clear();
            writes_seen.clear();
            reads.clear();
            writes.clear();
            for access in &func.decl.field_accesses {
                if !param_names.contains(access.receiver_param.as_str()) {
                    continue;
                }
                if access.is_write {
                    if writes_seen.insert(access.field_name.as_str()) {
                        writes.push(access.field_name.clone());
                    }
                } else if reads_seen.insert(access.field_name.as_str()) {
                    reads.push(access.field_name.clone());
                }
            }

            usages.push(ExternalFuncUsage {
                func,
                takes_mut_ref,
                direct_field_reads: std::mem::take(&mut reads),
                direct_field_writes: std::mem::take(&mut writes),
                other_project_type_params: other_project_types.len(),
            });
        }

        usages
    }

    /// Analyze where unit and integration tests exercising `type_id` are located.
    #[must_use]
    pub fn tests_for(&self, type_id: TypeId) -> TypeTestProfile<'_> {
        let ty = &self.types[type_id.0];
        let mut in_defining_module = Vec::new();
        let mut scattered_unit_tests = Vec::new();
        let mut integration_tests = Vec::new();

        for func in &self.functions {
            if !func.decl.is_test() {
                continue;
            }

            let constructs_type = func.resolved_constructions.contains(&type_id);
            let calls_type_helper = self.test_calls_type_helper(func, type_id);
            if !constructs_type && !calls_type_helper {
                continue;
            }

            if self.files[func.file_id.0].is_integration_test_file() {
                integration_tests.push(func);
            } else if func.module_path == ty.module_path {
                in_defining_module.push(func);
            } else {
                scattered_unit_tests.push(func);
            }
        }

        TypeTestProfile {
            in_defining_module,
            scattered_unit_tests,
            integration_tests,
        }
    }

    fn test_calls_type_helper(&self, func: &IndexedFunc, type_id: TypeId) -> bool {
        let file_imports = &self.file_imports_by_local_name[func.file_id.0];
        for call in &func.decl.calls {
            if call.receiver.is_some() {
                continue;
            }
            let Some(candidates) = self.free_funcs_by_name.get(&call.callee) else {
                continue;
            };
            for &cid in candidates {
                if !self.func_param_types[cid.0].contains(&type_id) {
                    continue;
                }
                let callee = &self.functions[cid.0];
                if callee.module_path == func.module_path || file_imports.contains(&call.callee) {
                    return true;
                }
            }
        }
        false
    }

    #[must_use]
    pub fn is_line_in_test_code(&self, file_id: FileId, line: usize) -> bool {
        let file = &self.files[file_id.0];
        if file.is_test_file() {
            return true;
        }
        self.functions.iter().any(|f| {
            f.file_id == file_id
                && f.decl.is_test_code()
                && line >= f.decl.span.start_line()
                && line <= f.decl.span.end_line()
        })
    }

    #[must_use]
    pub fn is_suppressed(&self, rule_id: &str, span: &Span) -> bool {
        let Some(&file_id) = self.file_by_rel_path.get(span.file()) else {
            return false;
        };
        self.files[file_id.0]
            .suppressions
            .iter()
            .any(|s| s.matches(rule_id, span.start_line()))
    }

    #[must_use]
    pub fn file_id_for_path(&self, rel_path: &Path) -> Option<FileId> {
        self.file_by_rel_path.get(rel_path).copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::ModulePath;
    use crate::ast::item::{
        CallKind, CallReceiver, CallSite, FieldAccess, FieldDecl, FunctionDecl, FunctionRole,
        ImportDecl, ImportKind, ParamDecl, TypeDecl, TypeKind, TypeRef, Visibility,
    };
    use crate::ast::vector::NormalizedAst;
    use crate::lang::rust::rust_language_spec;

    fn make_file(
        rel: &str,
        module: &[&str],
        types: Vec<TypeDecl>,
        functions: Vec<FunctionDecl>,
        imports: Vec<ImportDecl>,
    ) -> (SyntaxTree, FileAst) {
        let rust = rust_language_spec();
        let tree = SyntaxTree::parse(rel, rel, rust.clone(), "");
        let role = if rel.starts_with("tests/") {
            FileRole::IntegrationTest
        } else {
            FileRole::Module
        };
        let ast = FileAst {
            path: rel.into(),
            relative_path: rel.into(),
            module_path: ModulePath::new(module.iter().map(|s| (*s).to_string()).collect()),
            language: rust.id(),
            member_separator: rust.member_separator(),
            role,
            imports,
            exports: Vec::new(),
            types,
            functions,
            top_level_idents: Vec::new(),
            suppressions: Vec::new(),
        };
        (tree, ast)
    }

    #[test]
    fn resolves_external_free_functions_and_field_mutations_across_modules() {
        let order_type = TypeDecl {
            name: "Order".to_string(),
            kind: TypeKind::Struct,
            visibility: Visibility::Public,
            in_test_module: false,
            is_default_export: false,
            fields: vec![FieldDecl {
                name: "total".to_string(),
                type_name: Some(TypeRef::Named("u64".to_string())),
                visibility: Visibility::Public,
                is_mut_ref: false,
                span: Span::new("src/order.rs", 2, 5, 2, 19),
            }],
            annotations: Vec::new(),
            span: Span::new("src/order.rs", 1, 1, 3, 2),
        };

        let helper = FunctionDecl {
            name: "apply_discount".to_string(),
            owner_type: None,
            is_interface_impl: false,
            self_receiver: None,
            visibility: Visibility::Public,
            role: FunctionRole::Production,
            params: vec![ParamDecl {
                name: "order".to_string(),
                type_name: Some(TypeRef::Named("Ord".to_string())),
                access: ParamAccess::MutableRef,
            }],
            return_type: None,
            body_tokens: NormalizedAst::default(),
            field_accesses: vec![FieldAccess {
                receiver_param: "order".to_string(),
                field_name: "total".to_string(),
                is_write: true,
                span: Span::new("src/pricing.rs", 5, 5, 5, 20),
            }],
            calls: Vec::new(),
            constructed_types: Vec::new(),
            local_bindings: Vec::new(),
            field_inits: Vec::new(),
            referenced_idents: Vec::new(),
            scoped_ident_refs: Vec::new(),
            span: Span::new("src/pricing.rs", 4, 1, 6, 2),
        };

        let (t1, f1) = make_file(
            "src/order.rs",
            &["crate", "order"],
            vec![order_type],
            vec![],
            vec![],
        );
        let (t2, f2) = make_file(
            "src/pricing.rs",
            &["crate", "pricing"],
            vec![],
            vec![helper],
            vec![ImportDecl {
                resolved_module: Some(ModulePath::new(vec!["crate".into(), "order".into()])),
                module_segments: vec!["crate".into(), "order".into()],
                kind: ImportKind::Named,
                symbol: "Order".into(),
                alias: Some("Ord".into()),
                in_test_module: false,
                span: Span::new("src/pricing.rs", 1, 1, 1, 35),
            }],
        );

        let codebase = Codebase::build(vec![t1, t2], vec![f1, f2]);
        let order_id = codebase.types()[0].id;
        let usages = codebase.external_free_functions_for(order_id);

        assert_eq!(usages.len(), 1);
        assert!(usages[0].takes_mut_ref);
        assert!(usages[0].mutates_fields());
        assert_eq!(usages[0].direct_field_writes, vec!["total"]);
    }

    #[test]
    fn tests_for_does_not_conflate_same_named_methods_across_unrelated_types() {
        let ty_a = TypeDecl {
            name: "Alpha".to_string(),
            kind: TypeKind::Struct,
            visibility: Visibility::Private,
            in_test_module: false,
            is_default_export: false,
            fields: vec![],
            annotations: vec![],
            span: Span::new("src/a.rs", 1, 1, 1, 15),
        };
        let alpha_new = FunctionDecl {
            name: "new".to_string(),
            owner_type: Some("Alpha".to_string()),
            is_interface_impl: false,
            self_receiver: None,
            visibility: Visibility::Private,
            role: FunctionRole::Production,
            params: vec![],
            return_type: None,
            body_tokens: NormalizedAst::default(),
            field_accesses: vec![],
            calls: vec![],
            constructed_types: vec![],
            local_bindings: vec![],
            field_inits: vec![],
            referenced_idents: vec![],
            scoped_ident_refs: vec![],
            span: Span::new("src/a.rs", 2, 1, 2, 15),
        };
        let unrelated_test = FunctionDecl {
            name: "tests_beta".to_string(),
            owner_type: None,
            is_interface_impl: false,
            self_receiver: None,
            visibility: Visibility::Private,
            role: FunctionRole::Test,
            params: vec![],
            return_type: None,
            body_tokens: NormalizedAst::default(),
            field_accesses: vec![],
            calls: vec![CallSite {
                kind: CallKind::Scoped,
                callee: "new".to_string(),
                receiver: Some(CallReceiver::Named("Beta".to_string())),
                span: Span::new("src/b.rs", 5, 5, 5, 16),
            }],
            constructed_types: vec![TypeRef::Named("Beta".to_string())],
            local_bindings: vec![],
            field_inits: vec![],
            referenced_idents: vec![],
            scoped_ident_refs: vec![],
            span: Span::new("src/b.rs", 4, 1, 6, 2),
        };

        let (t1, f1) = make_file(
            "src/a.rs",
            &["crate", "a"],
            vec![ty_a],
            vec![alpha_new],
            vec![],
        );
        let (t2, f2) = make_file(
            "src/b.rs",
            &["crate", "b"],
            vec![],
            vec![unrelated_test],
            vec![],
        );

        let codebase = Codebase::build(vec![t1, t2], vec![f1, f2]);
        let profile = codebase.tests_for(codebase.types()[0].id);
        assert!(profile.scattered_unit_tests.is_empty());
    }
}
