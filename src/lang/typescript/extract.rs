use super::TsNode;
use super::body::TsBodyWalker;
use super::coupling;
use super::imports::{self, ExportClauseResult, ImportBatch, ModuleImportParser};
use super::syntax::{self, TestRunnerKind};
use crate::ast::item::{
    ConditionalGuard, ExportDecl, FieldDecl, FunctionDecl, FunctionRole, ImportDecl, InterfaceImpl,
    ModuleMock, ParamAccess, ParamDecl, SubclassPolicy, TypeDecl, TypeKind, TypeRef, Visibility,
};
use crate::ast::{FileAst, FileRole, ModulePath, SyntaxTree};
use crate::lang::common::FunctionHeader;
use std::collections::HashMap;
use std::path::Component;

#[derive(Clone, Copy)]
struct DeclMeta {
    visibility: Visibility,
    is_default_export: bool,
    in_test_context: bool,
}

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
            tree, module_path, base_dir, namespace_imports: HashMap::new(),
            imports: Vec::new(), exports: Vec::new(), types: Vec::new(),
            interface_impls: Vec::new(), functions: Vec::new(),
            top_level_idents: Vec::new(), module_mocks: Vec::new(),
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

        self.top_level_idents.sort_unstable();
        self.top_level_idents.dedup();

        let is_barrel =
            imports::is_index_barrel(self.tree.relative_path(), &self.imports, &self.exports);
        FileAst {
            path: self.tree.path().to_path_buf(),
            relative_path: self.tree.relative_path().to_path_buf(),
            module_path: self.module_path,
            language: self.tree.language_id(),
            member_separator: self.tree.language().member_separator(),
            role,
            is_barrel,
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
                self.extract_class(node, meta, outer_annotations);
            }
            "interface_declaration" => self.extract_interface(node, meta),
            "enum_declaration" => self.extract_enum(node, meta),
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
            self.extract_exported_value(&val, is_default, in_test_context, &outer_annotations);
        }
    }

    fn extract_exported_value(
        &mut self,
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
                self.extract_class(val, meta, outer_annotations);
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
                    ty.is_default_export = true;
                }
            }
        }
        for func in &mut self.functions {
            if func.owner_type.is_none() && func.name == symbol {
                func.visibility = Visibility::Public;
                if is_default {
                    func.is_default_export = true;
                }
            }
        }
    }

    fn extract_class(&mut self, node: &TsNode<'a>, meta: DeclMeta, outer_annotations: &[String]) {
        let Some(name) = node.field("name").map(|n| n.text().into_owned()) else {
            return;
        };
        let mut fields = Vec::new();
        let mut annotations = outer_annotations.to_vec();
        syntax::collect_decorators(node, &mut annotations);

        if let Some(body) = node.field("body") {
            for child in body.children() {
                match child.kind().as_ref() {
                    "public_field_definition" | "field_definition" => {
                        if let Some(field_name) = child.field("name").map(|n| n.text().into_owned())
                        {
                            fields.push(FieldDecl {
                                name: field_name,
                                type_name: child
                                    .field("type")
                                    .and_then(|t| syntax::extract_type_ref(&t)),
                                visibility: syntax::extract_visibility(&child),
                                explicit_visibility: syntax::has_explicit_visibility(&child),
                                is_mut_ref: false,
                                conditional_guard: ConditionalGuard::None,
                                span: self.tree.node_span(&child),
                            });
                        }
                    }
                    "method_definition" => {
                        if let Some(method) =
                            self.extract_method(&child, &name, meta.in_test_context, &mut fields)
                        {
                            self.functions.push(method);
                        }
                    }
                    _ => {}
                }
            }
        }

        let (base_types, impls) =
            coupling::extract_class_heritage(node, &name, meta.in_test_context, self.tree);
        self.interface_impls.extend(impls);
        let subclass_policy = if node.kind() == "abstract_class_declaration" {
            SubclassPolicy::Open
        } else {
            SubclassPolicy::Closed
        };
        self.types.push(TypeDecl {
            name,
            kind: TypeKind::Class,
            visibility: meta.visibility,
            in_test_module: meta.in_test_context,
            is_default_export: meta.is_default_export,
            has_implicit_field_readers: false,
            subclass_policy,
            base_types,
            fields,
            annotations,
            span: self.tree.node_span(node),
        });
    }

    fn extract_method(
        &self,
        node: &TsNode<'a>,
        owner_name: &str,
        in_test_context: bool,
        owner_fields: &mut Vec<FieldDecl>,
    ) -> Option<FunctionDecl> {
        let name = node.field("name")?.text().into_owned();
        let is_ctor = name == "constructor";
        let is_static = syntax::has_modifier_token(node, "static");
        let self_receiver = (!is_ctor && !is_static).then_some(ParamAccess::MutableRef);
        let return_type = if is_ctor {
            Some(TypeRef::SelfType)
        } else {
            node.field("return_type")
                .and_then(|t| syntax::extract_type_ref(&t))
        };
        let params = node
            .field("parameters")
            .map_or_else(Vec::new, |param_list| {
                self.extract_param_list(&param_list, is_ctor.then_some(owner_fields))
            });
        let role = if in_test_context {
            FunctionRole::TestHelper
        } else {
            FunctionRole::Production
        };
        let body = TsBodyWalker::new(self.tree, &self.base_dir, &self.namespace_imports)
            .extract(node.field("body"));

        Some(body.into_function_decl(
            FunctionHeader {
                name,
                owner_type: Some(owner_name.to_string()),
                is_interface_impl: false,
                interface_name: None,
                self_receiver,
                visibility: syntax::extract_visibility(node),
                explicit_visibility: syntax::has_explicit_visibility(node),
                is_default_export: false,
                role,
                params,
                return_type,
            },
            self.tree.node_span(node),
        ))
    }

    fn extract_interface(&mut self, node: &TsNode<'a>, meta: DeclMeta) {
        let Some(name) = node.field("name").map(|n| n.text().into_owned()) else {
            return;
        };
        let mut fields = Vec::new();
        if let Some(body) = node.field("body") {
            for child in body.children() {
                if child.kind() == "property_signature"
                    && let Some(prop_name) = child.field("name").map(|n| n.text().into_owned())
                {
                    fields.push(FieldDecl {
                        name: prop_name,
                        type_name: child
                            .field("type")
                            .and_then(|t| syntax::extract_type_ref(&t)),
                        visibility: Visibility::Public,
                        explicit_visibility: false,
                        is_mut_ref: false,
                        conditional_guard: ConditionalGuard::None,
                        span: self.tree.node_span(&child),
                    });
                }
            }
        }
        self.types.push(TypeDecl {
            name,
            kind: TypeKind::Interface,
            visibility: meta.visibility,
            in_test_module: meta.in_test_context,
            is_default_export: meta.is_default_export,
            has_implicit_field_readers: false,
            subclass_policy: SubclassPolicy::Closed,
            base_types: Vec::new(),
            fields,
            annotations: Vec::new(),
            span: self.tree.node_span(node),
        });
    }

    fn extract_enum(&mut self, node: &TsNode<'a>, meta: DeclMeta) {
        let Some(name) = node.field("name").map(|n| n.text().into_owned()) else {
            return;
        };
        self.types.push(TypeDecl {
            name,
            kind: TypeKind::Enum,
            visibility: meta.visibility,
            in_test_module: meta.in_test_context,
            is_default_export: meta.is_default_export,
            has_implicit_field_readers: false,
            subclass_policy: SubclassPolicy::Closed,
            base_types: Vec::new(),
            fields: Vec::new(),
            annotations: Vec::new(),
            span: self.tree.node_span(node),
        });
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
        let params = self.extract_callable_params(node);
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
                is_default_export: meta.is_default_export,
                role,
                params,
                return_type,
            },
            self.tree.node_span(node),
        ));
    }

    fn extract_callable_params(&self, node: &TsNode<'a>) -> Vec<ParamDecl> {
        if let Some(param_list) = node.field("parameters") {
            return self.extract_param_list(&param_list, None);
        }
        if let Some(single) = node.field("parameter")
            && single.kind() == "identifier"
        {
            return vec![ParamDecl {
                name: single.text().into_owned(),
                type_name: None,
                access: ParamAccess::Owned,
            }];
        }
        Vec::new()
    }

    fn extract_param_list(
        &self,
        param_list: &TsNode<'a>,
        mut ctor_fields: Option<&mut Vec<FieldDecl>>,
    ) -> Vec<ParamDecl> {
        let mut params = Vec::new();
        for child in param_list.children() {
            if !matches!(
                child.kind().as_ref(),
                "required_parameter" | "optional_parameter"
            ) {
                continue;
            }
            let Some(param_name) = syntax::extract_param_name(&child) else {
                continue;
            };
            let type_name = child
                .field("type")
                .and_then(|t| syntax::extract_type_ref(&t));
            if let Some(fields) = ctor_fields.as_deref_mut()
                && syntax::is_parameter_property(&child)
            {
                fields.push(FieldDecl {
                    name: param_name.clone(),
                    type_name: type_name.clone(),
                    visibility: syntax::extract_visibility(&child),
                    explicit_visibility: syntax::has_explicit_visibility(&child),
                    is_mut_ref: false,
                    conditional_guard: ConditionalGuard::None,
                    span: self.tree.node_span(&child),
                });
            }
            params.push(ParamDecl {
                name: param_name,
                type_name,
                access: ParamAccess::Owned,
            });
        }
        params
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
                let params = self.extract_callable_params(&cb);
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
                        is_default_export: false,
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
