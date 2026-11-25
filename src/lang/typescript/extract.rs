use super::TsNode;
use super::body::TsBodyWalker;
use super::coupling;
use super::imports::{self, ExportClauseResult, ImportBatch, ModuleImportParser};
use super::members::{DeclMeta, ExtractedClass, TsMemberExtractor};
use super::syntax::{self, TestRunnerKind};
use crate::ast::item::{
    ExportBinding, ExportDecl, ExportOrigin, FunctionDecl, FunctionRole, ImportDecl, InterfaceImpl, ModuleMock,
    TypeDecl, Visibility,
};
use crate::ast::{FileAst, FileRole, ModulePath, SyntaxTree};
use crate::lang::common::FunctionHeader;
use std::collections::{HashMap, HashSet};
use std::path::Component;

pub(super) struct TypeScriptExtractor<'a> {
    tree: &'a SyntaxTree,
    module_path: ModulePath,
    base_dir: Vec<String>,
    namespace_imports: HashMap<String, ModulePath>,
    imports: Vec<ImportDecl>,
    exports: Vec<ExportDecl>,
    types: Vec<TypeDecl>,
    interface_impls: Vec<InterfaceImpl>,
    functions: Vec<FunctionDecl>,
    top_level_idents: Vec<String>,
    module_mocks: Vec<ModuleMock>,
}

impl<'a> TypeScriptExtractor<'a> {
    pub(super) fn new(tree: &'a SyntaxTree, module_path: ModulePath) -> Self {
        let base_dir = tree.relative_path().parent().map_or_else(Vec::new, |dir| {
            dir.components()
                .filter_map(|c| match c {
                    Component::Normal(seg) => seg.to_str().map(String::from),
                    _ => None,
                })
                .collect()
        });
        Self {
            tree,
            module_path,
            base_dir,
            namespace_imports: HashMap::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            types: Vec::new(),
            interface_impls: Vec::new(),
            functions: Vec::new(),
            top_level_idents: Vec::new(),
            module_mocks: Vec::new(),
        }
    }

    pub(super) fn extract_file(mut self, role: FileRole) -> FileAst {
        let in_test_file = role.is_test_file();
        let root = self.tree.root();

        for child in root.children() {
            if child.kind() == "import_statement" {
                let ImportBatch {
                    imports,
                    namespace_imports,
                } = ModuleImportParser::new(self.tree, &self.base_dir)
                    .parse_import(&child, in_test_file);
                self.imports.extend(imports);
                for (name, target) in namespace_imports {
                    self.namespace_imports.insert(name, target);
                }
            }
        }
        for child in root.children() {
            if child.kind() != "import_statement" {
                self.walk_statement(&child, in_test_file);
            }
        }

        let imported_locals: HashSet<&str> = self
            .imports
            .iter()
            .filter(|imp| !imp.in_test_module)
            .map(ImportDecl::local_name)
            .collect();
        for exp in &mut self.exports {
            if exp.origin == ExportOrigin::Local && imported_locals.contains(exp.symbol.as_str()) {
                exp.origin = ExportOrigin::ImportedBinding;
            }
        }

        self.top_level_idents.sort_unstable();
        self.top_level_idents.dedup();

        let barrel_kind = imports::classify_barrel_kind(
            self.tree,
            &self.imports,
            &self.exports,
            "index",
            ".d.ts",
        );
        FileAst {
            path: self.tree.path().to_path_buf(),
            relative_path: self.tree.relative_path().to_path_buf(),
            module_path: self.module_path,
            language: self.tree.language_id(),
            member_separator: self.tree.language().member_separator(),
            role,
            barrel_kind,
            module_decls: Vec::new(),
            imports: self.imports,
            exports: self.exports,
            types: self.types,
            interface_impls: self.interface_impls,
            functions: self.functions,
            top_level_idents: self.top_level_idents,
            test_hooks: Vec::new(),
            module_mocks: self.module_mocks,
            suppressions: self.tree.extract_suppressions(),
        }
    }

    fn member_extractor(&self) -> TsMemberExtractor<'a, '_> {
        TsMemberExtractor::new(self.tree, &self.base_dir, &self.namespace_imports)
    }

    fn walk_statement(&mut self, node: &TsNode<'a>, in_test_context: bool) {
        let private_meta = DeclMeta {
            visibility: Visibility::Private,
            is_default_export: false,
            in_test_context,
        };
        match node.kind().as_ref() {
            "export_statement" => self.extract_export(node, in_test_context),
            "expression_statement" => self.extract_expression_statement(node, in_test_context),
            _ => self.extract_declaration(node, private_meta, &[]),
        }
    }

    fn extract_declaration(
        &mut self,
        node: &TsNode<'a>,
        meta: DeclMeta,
        outer_annotations: &[String],
    ) {
        match node.kind().as_ref() {
            "class_declaration" | "abstract_class_declaration" | "class" => {
                if let Some(ExtractedClass {
                    decl,
                    methods,
                    interface_impls,
                }) = self
                    .member_extractor()
                    .extract_class(node, meta, outer_annotations)
                {
                    self.types.push(decl);
                    self.functions.extend(methods);
                    self.interface_impls.extend(interface_impls);
                }
            }
            "interface_declaration" => {
                if let Some(decl) = self.member_extractor().extract_interface(node, meta) {
                    self.types.push(decl);
                }
            }
            "enum_declaration" => {
                if let Some(decl) = self.member_extractor().extract_enum(node, meta) {
                    self.types.push(decl);
                }
            }
            "function_declaration" => {
                if let Some(name) = node.field("name").map(|n| n.text().into_owned()) {
                    self.extract_function_like(node, name, meta);
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                self.extract_variable_declaration(node, meta);
            }
            _ => {}
        }
    }

    fn extract_export(&mut self, node: &TsNode<'a>, in_test_context: bool) {
        let is_default = syntax::has_modifier_token(node, "default");
        let mut outer_annotations = Vec::new();
        syntax::collect_decorators(node, &mut outer_annotations);

        if let Some(decl) = node.field("declaration") {
            let meta = DeclMeta {
                visibility: Visibility::Public,
                is_default_export: is_default,
                in_test_context,
            };
            self.extract_declaration(&decl, meta, &outer_annotations);
            return;
        }

        if let Some(ExportClauseResult {
            exports,
            local_exported,
        }) = ModuleImportParser::new(self.tree, &self.base_dir).parse_reexport(node)
        {
            self.exports.extend(exports);
            for (symbol, spec_default) in local_exported {
                self.mark_local_exported(&symbol, spec_default);
            }
            return;
        }

        if let Some(val) = node.field("value") {
            self.extract_exported_value(
                node,
                &val,
                is_default,
                in_test_context,
                &outer_annotations,
            );
        }
    }

    fn extract_exported_value(
        &mut self,
        export_node: &TsNode<'a>,
        val: &TsNode<'a>,
        is_default: bool,
        in_test_context: bool,
        outer_annotations: &[String],
    ) {
        let meta = DeclMeta {
            visibility: Visibility::Public,
            is_default_export: is_default,
            in_test_context,
        };
        match val.kind().as_ref() {
            "class" | "class_declaration" | "abstract_class_declaration" => {
                if let Some(ExtractedClass {
                    decl,
                    methods,
                    interface_impls,
                }) = self
                    .member_extractor()
                    .extract_class(val, meta, outer_annotations)
                {
                    self.types.push(decl);
                    self.functions.extend(methods);
                    self.interface_impls.extend(interface_impls);
                }
            }
            "function_declaration" | "function_expression" => {
                let name = val
                    .field("name")
                    .map_or_else(|| String::from("default"), |n| n.text().into_owned());
                self.extract_function_like(val, name, meta);
            }
            "arrow_function" => self.extract_function_like(val, String::from("default"), meta),
            "identifier" => {
                let sym = val.text().into_owned();
                if is_default {
                    self.exports.push(ExportDecl {
                        source_module: None,
                        symbol: sym.clone(),
                        exported_name: String::from("default"),
                        is_default: true,
                        is_wildcard: false,
                        visibility: Visibility::Public,
                        origin: ExportOrigin::Local,
                        span: self.tree.node_span(export_node),
                    });
                }
                self.mark_local_exported(&sym, is_default);
            }
            _ => {
                if !in_test_context {
                    syntax::collect_leaf_idents(val, &mut self.top_level_idents);
                }
            }
        }
    }

    fn mark_local_exported(&mut self, symbol: &str, is_default: bool) {
        for ty in &mut self.types {
            if ty.name == symbol {
                ty.visibility = Visibility::Public;
                if is_default {
                    ty.export_binding = ExportBinding::Default;
                }
            }
        }
        for func in &mut self.functions {
            if func.owner_type.is_none() && func.name == symbol {
                func.visibility = Visibility::Public;
                if is_default {
                    func.export_binding = ExportBinding::Default;
                }
            }
        }
    }

    fn extract_variable_declaration(&mut self, node: &TsNode<'a>, meta: DeclMeta) {
        for child in node.children() {
            if child.kind() != "variable_declarator" {
                continue;
            }
            let Some(val) = child.field("value") else {
                continue;
            };
            if matches!(
                val.kind().as_ref(),
                "arrow_function" | "function_expression"
            ) && let Some(name_node) = child.field("name")
                && name_node.kind() == "identifier"
            {
                self.extract_function_like(&val, name_node.text().into_owned(), meta);
            } else if !meta.in_test_context {
                syntax::collect_leaf_idents(&val, &mut self.top_level_idents);
            }
        }
    }

    fn extract_function_like(&mut self, node: &TsNode<'a>, name: String, meta: DeclMeta) {
        let params = self.member_extractor().extract_callable_params(node);
        let return_type = node
            .field("return_type")
            .and_then(|t| syntax::extract_type_ref(&t));
        let role = if meta.in_test_context {
            FunctionRole::TestHelper
        } else if name == "main" {
            FunctionRole::Entrypoint
        } else {
            FunctionRole::Production
        };
        let body = TsBodyWalker::new(self.tree, &self.base_dir, &self.namespace_imports)
            .extract(node.field("body"));

        self.functions.push(body.into_function_decl(
            FunctionHeader {
                name,
                owner_type: None,
                is_interface_impl: false,
                interface_name: None,
                self_receiver: None,
                visibility: meta.visibility,
                explicit_visibility: meta.visibility == Visibility::Public,
                export_binding: ExportBinding::from_is_default(meta.is_default_export),
                is_lifecycle: false,
                role,
                params,
                return_type,
            },
            self.tree.node_span(node),
        ));
    }

    fn extract_expression_statement(&mut self, node: &TsNode<'a>, in_test_context: bool) {
        let Some(expr) = node.children().find(TsNode::is_named) else {
            return;
        };
        if in_test_context && expr.kind() == "call_expression" {
            if let Some(mock) = coupling::detect_module_mock(self.tree, &self.base_dir, &expr, true)
            {
                self.module_mocks.push(mock);
                return;
            }
            if let Some(runner_kind) = TestRunnerKind::from_call(&expr) {
                self.extract_test_runner_call(&expr, runner_kind);
                return;
            }
        }
        if !in_test_context {
            syntax::collect_leaf_idents(&expr, &mut self.top_level_idents);
        }
    }

    fn extract_test_runner_call(&mut self, call_node: &TsNode<'a>, kind: TestRunnerKind) {
        let Some(args) = call_node.field("arguments") else {
            return;
        };
        let mut title: Option<String> = None;
        let mut callback = None;

        for arg in args.children() {
            if !arg.is_named() {
                continue;
            }
            if title.is_none() && matches!(arg.kind().as_ref(), "string" | "template_string") {
                let cleaned = syntax::unquote_string(&arg.text()).trim().to_string();
                if !cleaned.is_empty() {
                    title = Some(cleaned);
                }
            } else if matches!(
                arg.kind().as_ref(),
                "arrow_function" | "function_expression"
            ) {
                callback = Some(arg);
            }
        }

        let Some(cb) = callback else {
            return;
        };
        match kind {
            TestRunnerKind::Suite => {
                if let Some(body) = cb.field("body") {
                    for stmt in body.children() {
                        self.walk_statement(&stmt, true);
                    }
                }
            }
            TestRunnerKind::Case => {
                let name = title.unwrap_or_else(|| String::from("anonymous_test"));
                let params = self.member_extractor().extract_callable_params(&cb);
                let body = TsBodyWalker::new(self.tree, &self.base_dir, &self.namespace_imports)
                    .extract(cb.field("body"));
                self.functions.push(body.into_function_decl(
                    FunctionHeader {
                        name,
                        owner_type: None,
                        is_interface_impl: false,
                        interface_name: None,
                        self_receiver: None,
                        visibility: Visibility::Private,
                        explicit_visibility: false,
                        export_binding: ExportBinding::Standard,
                        is_lifecycle: false,
                        role: FunctionRole::Test,
                        params,
                        return_type: None,
                    },
                    self.tree.node_span(call_node),
                ));
            }
        }
    }
}
