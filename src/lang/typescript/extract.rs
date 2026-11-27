use super::TsNode;
use super::body::TsBodyWalker;
use super::components::{self, CallableSignature, PendingExportCall};
use super::conventions::TypeScriptConventions;
use super::coupling;
use super::expr::TsBodyContext;
use super::imports::{self, ExportClauseResult, ImportBatch, ModuleImportParser};
use super::members::{DeclMeta, ExtractedClass, TsMemberExtractor};
use super::props::{self, PropShapeMap};
use super::syntax::{self, TestRunnerKind};
use crate::ast::item::{
    ExportBinding, ExportDecl, FunctionDecl, FunctionRole, ImportDecl, InterfaceImpl, ModuleMock,
    TypeDecl, Visibility,
};
use crate::ast::{FileAst, FileRole, ModulePath, SyntaxTree};
use crate::diagnostic::Span;
use crate::lang::common::FunctionHeader;
use std::collections::{HashMap, HashSet};

#[derive(Default)]
struct ExtractedItems<'a> {
    namespace_imports: HashMap<String, ModulePath>,
    prop_shapes: PropShapeMap,
    imports: Vec<ImportDecl>,
    exports: Vec<ExportDecl>,
    types: Vec<TypeDecl>,
    interface_impls: Vec<InterfaceImpl>,
    functions: Vec<FunctionDecl>,
    top_level_idents: Vec<String>,
    module_mocks: Vec<ModuleMock>,
    has_value_export: bool,
    pending_local_exports: Vec<(String, bool)>,
    pending_export_calls: Vec<PendingExportCall<'a>>,
}

pub(super) struct TypeScriptExtractor<'a> {
    tree: &'a SyntaxTree,
    module_path: ModulePath,
    conventions: &'a TypeScriptConventions,
    base_dir: Vec<String>,
    items: ExtractedItems<'a>,
}

impl<'a> TypeScriptExtractor<'a> {
    pub(super) fn new(
        tree: &'a SyntaxTree,
        module_path: ModulePath,
        conventions: &'a TypeScriptConventions,
    ) -> Self {
        let base_dir = components::base_dir_segments(tree);
        Self {
            tree,
            module_path,
            conventions,
            base_dir,
            items: ExtractedItems::default(),
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
                } = ModuleImportParser::new(self.tree, &self.base_dir, self.conventions)
                    .parse_import(&child, in_test_file);
                self.items.imports.extend(imports);
                self.items.namespace_imports.extend(namespace_imports);
            }
        }
        self.items.prop_shapes = props::collect_prop_shapes(&root, self.conventions);

        for child in root.children() {
            if child.kind() != "import_statement" {
                self.walk_statement(&child, in_test_file);
            }
        }
        self.apply_pending_local_exports();

        components::reclassify_imported_exports(&self.items.imports, &mut self.items.exports);
        self.items.top_level_idents.sort_unstable();
        self.items.top_level_idents.dedup();

        let has_exported_items = self.items.has_value_export
            || !self.items.exports.is_empty()
            || self
                .items
                .types
                .iter()
                .any(|t| t.visibility == Visibility::Public)
            || self
                .items
                .functions
                .iter()
                .any(|f| f.owner_type.is_none() && f.visibility == Visibility::Public);
        let role = components::finalize_file_role(
            role,
            has_exported_items,
            !self.items.top_level_idents.is_empty(),
        );

        let barrel_kind = imports::classify_barrel_kind(
            self.tree,
            &self.items.imports,
            &self.items.exports,
            self.conventions,
        );
        FileAst {
            path: self.tree.path().to_path_buf(),
            relative_path: self.tree.relative_path().to_path_buf(),
            module_path: self.module_path,
            language: self.tree.semantic_language_id(),
            member_separator: self.tree.language().member_separator(),
            role,
            barrel_kind,
            module_decls: Vec::new(),
            imports: self.items.imports,
            exports: self.items.exports,
            types: self.items.types,
            interface_impls: self.items.interface_impls,
            functions: self.items.functions,
            top_level_idents: self.items.top_level_idents,
            test_hooks: Vec::new(),
            module_mocks: self.items.module_mocks,
            suppressions: self.tree.extract_suppressions(),
        }
    }

    fn body_ctx(&self) -> TsBodyContext<'a, '_> {
        TsBodyContext::new(
            self.tree,
            &self.base_dir,
            &self.items.namespace_imports,
            self.conventions,
            &self.items.prop_shapes,
        )
    }

    fn push_extracted_class(&mut self, extracted: Option<ExtractedClass>) {
        if let Some(ExtractedClass {
            decl,
            methods,
            interface_impls,
        }) = extracted
        {
            self.items.types.push(decl);
            self.items.functions.extend(methods);
            self.items.interface_impls.extend(interface_impls);
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
            "class_declaration"
            | "abstract_class_declaration"
            | "class"
            | "interface_declaration"
            | "enum_declaration"
            | "function_declaration"
            | "lexical_declaration"
            | "variable_declaration" => self.extract_declaration(node, private_meta, &[]),
            "type_alias_declaration"
            | "ambient_declaration"
            | "import_statement"
            | "empty_statement"
            | "comment"
            | "debugger_statement" => {}
            _ if !in_test_context => self.record_top_level_fallback(node),
            _ => {}
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
                let extracted = TsMemberExtractor::new(&self.body_ctx()).extract_class(
                    node,
                    meta,
                    outer_annotations,
                );
                self.push_extracted_class(extracted);
            }
            "interface_declaration" => {
                if let Some(decl) =
                    TsMemberExtractor::new(&self.body_ctx()).extract_interface(node, meta)
                {
                    self.items.types.push(decl);
                }
            }
            "enum_declaration" => {
                if let Some(decl) =
                    TsMemberExtractor::new(&self.body_ctx()).extract_enum(node, meta)
                {
                    self.items.types.push(decl);
                }
            }
            "function_declaration" => {
                if let Some(name) = node.field("name").map(|n| n.text().into_owned()) {
                    let span = self.tree.node_span(node);
                    self.push_function(node, name, meta, None, span);
                }
            }
            "lexical_declaration" | "variable_declaration" => {
                self.extract_variable_declaration(node, meta);
            }
            _ => {}
        }
    }

    fn extract_export(&mut self, node: &TsNode<'a>, in_test_context: bool) {
        if !in_test_context && !components::is_type_only_export(node) {
            self.items.has_value_export = true;
        }
        let is_default = syntax::has_modifier_token(node, "default");
        let mut outer_annotations = Vec::new();
        syntax::collect_decorators(node, &mut outer_annotations);
        let meta = DeclMeta {
            visibility: Visibility::Public,
            is_default_export: is_default,
            in_test_context,
        };

        if let Some(decl) = node.field("declaration") {
            self.extract_declaration(&decl, meta, &outer_annotations);
            return;
        }
        if let Some(ExportClauseResult {
            exports,
            local_exported,
        }) = ModuleImportParser::new(self.tree, &self.base_dir, self.conventions)
            .parse_reexport(node)
        {
            self.items.exports.extend(exports);
            self.items.pending_local_exports.extend(local_exported);
            return;
        }
        if let Some(val) = node.field("value") {
            self.extract_exported_value(node, &val, meta, &outer_annotations);
        }
    }

    fn extract_exported_value(
        &mut self,
        export_node: &TsNode<'a>,
        val: &TsNode<'a>,
        meta: DeclMeta,
        outer_annotations: &[String],
    ) {
        let export_span = self.tree.node_span(export_node);
        match val.kind().as_ref() {
            "class" | "class_declaration" | "abstract_class_declaration" => {
                let extracted = TsMemberExtractor::new(&self.body_ctx()).extract_class(
                    val,
                    meta,
                    outer_annotations,
                );
                self.push_extracted_class(extracted);
            }
            "function_declaration" | "function_expression" | "arrow_function" => {
                let name = val
                    .field("name")
                    .map_or_else(|| String::from("default"), |n| n.text().into_owned());
                self.push_function(val, name, meta, None, export_span);
            }
            "identifier" => {
                let sym = val.text().into_owned();
                if meta.is_default_export {
                    self.items
                        .exports
                        .push(components::local_default_export_decl(
                            sym.clone(),
                            export_span,
                        ));
                }
                self.items
                    .pending_local_exports
                    .push((sym, meta.is_default_export));
            }
            "call_expression" => {
                if self.try_extract_wrapped_component(val, None, meta, None, export_span.clone()) {
                    return;
                }
                if meta.is_default_export {
                    self.items.pending_export_calls.push(PendingExportCall {
                        call_node: val.clone(),
                        span: export_span,
                        in_test_context: meta.in_test_context,
                    });
                } else if !meta.in_test_context {
                    self.record_top_level_fallback(val);
                }
            }
            _ if !meta.in_test_context => self.record_top_level_fallback(val),
            _ => {}
        }
    }

    fn try_extract_wrapped_component(
        &mut self,
        call_node: &TsNode<'a>,
        explicit_name: Option<String>,
        meta: DeclMeta,
        decl_type_hint: Option<TsNode<'a>>,
        span: Span,
    ) -> bool {
        let Some(unwrapped) = components::unwrap_component_wrapper(call_node, self.conventions)
        else {
            return false;
        };
        let name = explicit_name.unwrap_or_else(|| {
            unwrapped
                .func_node
                .field("name")
                .map_or_else(|| String::from("default"), |n| n.text().into_owned())
        });
        let hint = decl_type_hint.or(unwrapped.prop_type_hint);
        self.push_function(&unwrapped.func_node, name, meta, hint.as_ref(), span);
        if !meta.in_test_context {
            for extra in &unwrapped.extra_args {
                syntax::collect_leaf_idents(extra, None, &mut self.items.top_level_idents);
            }
        }
        true
    }

    fn record_top_level_fallback(&mut self, node: &TsNode<'a>) {
        syntax::collect_leaf_idents(node, None, &mut self.items.top_level_idents);
        for (decl, ident) in components::collect_top_level_dynamic_imports(node, &self.body_ctx()) {
            self.items.imports.push(decl);
            self.items.top_level_idents.push(ident);
        }
    }

    fn apply_pending_local_exports(&mut self) {
        for (symbol, is_default) in std::mem::take(&mut self.items.pending_local_exports) {
            components::mark_exported_types(&mut self.items.types, &symbol, is_default);
            components::mark_exported_functions(&mut self.items.functions, &symbol, is_default);
        }
        let export_calls = std::mem::take(&mut self.items.pending_export_calls);
        if export_calls.is_empty() {
            return;
        }
        let local_names: HashSet<String> = self
            .items
            .functions
            .iter()
            .filter(|f| f.owner_type.is_none())
            .map(|f| f.name.clone())
            .chain(self.items.types.iter().map(|t| t.name.clone()))
            .collect();

        for pending in export_calls {
            if let Some((target_name, target_range)) =
                components::find_exported_call_target(&pending.call_node, &local_names)
            {
                self.items
                    .exports
                    .push(components::local_default_export_decl(
                        target_name.clone(),
                        pending.span,
                    ));
                components::mark_exported_types(&mut self.items.types, &target_name, true);
                components::mark_exported_functions(&mut self.items.functions, &target_name, true);
                if !pending.in_test_context {
                    syntax::collect_leaf_idents(
                        &pending.call_node,
                        Some(&target_range),
                        &mut self.items.top_level_idents,
                    );
                }
            } else if !pending.in_test_context {
                self.record_top_level_fallback(&pending.call_node);
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
            let name_ident = child
                .field("name")
                .filter(|n| n.kind() == "identifier")
                .map(|n| n.text().into_owned());
            let decl_type_hint = child
                .field("type")
                .and_then(|t| props::extract_component_type_hint(&t, self.conventions));
            let span = self.tree.node_span(&child);

            if matches!(
                val.kind().as_ref(),
                "arrow_function" | "function_expression"
            ) && let Some(name) = name_ident
            {
                self.push_function(&val, name, meta, decl_type_hint.as_ref(), span);
                continue;
            }
            if val.kind() == "call_expression"
                && let Some(name) = name_ident
            {
                if self.try_extract_wrapped_component(
                    &val,
                    Some(name.clone()),
                    meta,
                    decl_type_hint,
                    span.clone(),
                ) {
                    continue;
                }
                if !meta.in_test_context
                    && let Some(target_mod) =
                        components::extract_lazy_import_target(&val, &self.body_ctx())
                {
                    let (import, export) = components::build_lazy_import_and_export(
                        name,
                        &target_mod,
                        meta.visibility,
                        meta.is_default_export,
                        span,
                    );
                    self.items.imports.push(import);
                    if let Some(exp) = export {
                        self.items.exports.push(exp);
                    }
                    continue;
                }
            }
            if !meta.in_test_context {
                self.record_top_level_fallback(&val);
            }
        }
    }

    fn push_function(
        &mut self,
        node: &TsNode<'a>,
        name: String,
        meta: DeclMeta,
        prop_type_hint: Option<&TsNode<'a>>,
        span: Span,
    ) {
        let role = if meta.in_test_context {
            FunctionRole::TestHelper
        } else if name == "main" {
            FunctionRole::Entrypoint
        } else {
            FunctionRole::Production
        };
        let return_type = node
            .field("return_type")
            .and_then(|t| syntax::extract_type_ref(&t));
        self.record_callable(
            node,
            CallableSignature {
                name,
                meta,
                role,
                return_type,
            },
            prop_type_hint,
            span,
        );
    }

    fn record_callable(
        &mut self,
        node: &TsNode<'a>,
        sig: CallableSignature,
        prop_type_hint: Option<&TsNode<'a>>,
        span: Span,
    ) {
        let CallableSignature {
            name,
            meta,
            role,
            return_type,
        } = sig;
        let func = {
            let ctx = self.body_ctx();
            let extracted =
                TsMemberExtractor::new(&ctx).extract_callable_params(node, prop_type_hint);
            let body = TsBodyWalker::new(&ctx).extract_with_seeds(
                node.field("body"),
                extracted.local_bindings,
                &extracted.default_exprs,
            );
            body.into_function_decl(
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
                    params: extracted.params,
                    return_type,
                },
                span,
            )
        };
        self.items.functions.push(func);
    }

    fn extract_expression_statement(&mut self, node: &TsNode<'a>, in_test_context: bool) {
        let Some(expr_node) = node.children().find(TsNode::is_named) else {
            return;
        };
        if in_test_context && expr_node.kind() == "call_expression" {
            if let Some(mock) = coupling::detect_module_mock(
                self.tree,
                &self.base_dir,
                self.conventions,
                &expr_node,
                true,
            ) {
                self.items.module_mocks.push(mock);
                return;
            }
            if let Some(runner_kind) = TestRunnerKind::from_call(&expr_node) {
                self.extract_test_runner_call(&expr_node, runner_kind);
                return;
            }
        }
        if !in_test_context {
            self.record_top_level_fallback(&expr_node);
        }
    }

    fn extract_test_runner_call(&mut self, call_node: &TsNode<'a>, kind: TestRunnerKind) {
        let Some((title, cb)) = syntax::extract_test_runner_args(call_node) else {
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
                let span = self.tree.node_span(call_node);
                let meta = DeclMeta {
                    visibility: Visibility::Private,
                    is_default_export: false,
                    in_test_context: true,
                };
                let name = title.unwrap_or_else(|| String::from("anonymous_test"));
                self.record_callable(
                    &cb,
                    CallableSignature {
                        name,
                        meta,
                        role: FunctionRole::Test,
                        return_type: None,
                    },
                    None,
                    span,
                );
            }
        }
    }
}
