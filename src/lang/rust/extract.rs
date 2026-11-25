use super::TsNode;
use super::body::BodyWalker;
use super::syntax;
use crate::ast::item::{
    ConditionalGuard, ExportBinding, ExportDecl, ExportOrigin, FieldDecl, FunctionDecl, FunctionRole, ImportDecl, ImportKind,
    InterfaceImpl, ModuleDecl, ParamAccess, SubclassPolicy, TypeDecl, TypeKind, TypeRef,
    Visibility,
};
use crate::ast::{BarrelKind, FileAst, FileRole, ModulePath, SyntaxTree};
use crate::diagnostic::Span;
use crate::lang::common::FunctionHeader;
use std::collections::HashSet;

struct UseContext<'a> {
    in_test_module: bool,
    reexport_visibility: Option<Visibility>,
    span: &'a Span,
}

pub(super) struct RustExtractor<'a> {
    tree: &'a SyntaxTree,
    module_path: ModulePath,
    module_decls: Vec<ModuleDecl>,
    declared_modules: HashSet<String>,
    imports: Vec<ImportDecl>,
    exports: Vec<ExportDecl>,
    types: Vec<TypeDecl>,
    interface_impls: Vec<InterfaceImpl>,
    functions: Vec<FunctionDecl>,
    top_level_idents: Vec<String>,
}

impl<'a> RustExtractor<'a> {
    pub(super) fn new(tree: &'a SyntaxTree, module_path: ModulePath) -> Self {
        Self {
            tree, module_path, module_decls: Vec::new(), declared_modules: HashSet::new(),
            imports: Vec::new(), exports: Vec::new(), types: Vec::new(),
            interface_impls: Vec::new(), functions: Vec::new(), top_level_idents: Vec::new(),
        }
    }

    pub(super) fn extract_file(mut self, role: FileRole) -> FileAst {
        let root = self.tree.root();
        self.module_decls = syntax::collect_top_level_mods(self.tree, &root);
        self.declared_modules = self.module_decls.iter().map(|m| m.name.clone()).collect();
        self.walk_item_list(&root, false);
        self.top_level_idents.sort_unstable();
        self.top_level_idents.dedup();

        FileAst {
            path: self.tree.path().to_path_buf(),
            relative_path: self.tree.relative_path().to_path_buf(),
            module_path: self.module_path,
            language: self.tree.language_id(),
            member_separator: self.tree.language().member_separator(),
            role,
            barrel_kind: BarrelKind::None,
            module_decls: self.module_decls,
            imports: self.imports,
            exports: self.exports,
            types: self.types,
            interface_impls: self.interface_impls,
            functions: self.functions,
            top_level_idents: self.top_level_idents,
            test_hooks: Vec::new(),
            module_mocks: Vec::new(),
            suppressions: self.tree.extract_suppressions(),
        }
    }

    fn walk_item_list(&mut self, container: &TsNode<'a>, in_test_module: bool) {
        let mut pending_attrs: Vec<String> = Vec::new();
        let mut pending_has_cfg_test = false;
        let mut compact_attr = String::new();

        for child in container.children() {
            let kind = child.kind();
            let item_in_test = in_test_module || pending_has_cfg_test;
            match kind.as_ref() {
                "attribute_item" => {
                    let text = child.text();
                    compact_attr.clear();
                    compact_attr.extend(text.chars().filter(|c| !c.is_whitespace()));
                    if compact_attr.contains("cfg(test)") {
                        pending_has_cfg_test = true;
                    }
                    pending_attrs.push(text.into_owned());
                    continue;
                }
                "line_comment" | "block_comment" => continue,
                "use_declaration" => self.extract_use(&child, item_in_test),
                "struct_item" => {
                    if let Some(ty) = self.extract_struct(&child, &pending_attrs, item_in_test) {
                        self.types.push(ty);
                    }
                }
                "enum_item" => {
                    if let Some(ty) = self.extract_enum(&child, &pending_attrs, item_in_test) {
                        self.types.push(ty);
                    }
                }
                "trait_item" => self.extract_trait(&child, &pending_attrs, item_in_test),
                "impl_item" => self.extract_impl(&child, item_in_test),
                "function_item" => {
                    if let Some(func) =
                        self.extract_function(&child, &pending_attrs, None, None, item_in_test)
                    {
                        self.functions.push(func);
                    }
                }
                "const_item" | "static_item" => {
                    if !item_in_test && let Some(val) = child.field("value") {
                        syntax::collect_leaf_idents(&val, &mut self.top_level_idents);
                    }
                }
                "mod_item" => {
                    let mod_name = child
                        .field("name")
                        .map(|n| n.text().into_owned())
                        .unwrap_or_default();
                    let child_is_test = item_in_test || mod_name == "tests";
                    if let Some(body) = child.field("body") {
                        self.walk_item_list(&body, child_is_test);
                    }
                }
                _ => {}
            }
            pending_attrs.clear();
            pending_has_cfg_test = false;
        }
    }

    fn extract_use(&mut self, node: &TsNode<'a>, in_test_module: bool) {
        if let Some(arg) = node.field("argument") {
            let span = self.tree.node_span(node);
            let vis = syntax::extract_visibility(node);
            let ctx = UseContext {
                in_test_module,
                reexport_visibility: (!in_test_module && vis.is_externally_visible())
                    .then_some(vis),
                span: &span,
            };
            let mut prefix = Vec::new();
            self.collect_use_tree(&arg, &mut prefix, &ctx);
        }
    }

    fn record_import(
        &mut self,
        raw_segments: Vec<String>,
        kind: ImportKind,
        symbol: String,
        alias: Option<String>,
        ctx: &UseContext<'_>,
    ) {
        let resolved_module = syntax::resolve_rust_import_module(
            &self.module_path,
            &self.declared_modules,
            &raw_segments,
            ctx.in_test_module,
        );
        let origin =
            syntax::classify_import_origin(&raw_segments, resolved_module.as_ref(), &symbol);
        let module_segments = match &resolved_module {
            Some(resolved) => resolved.segments().to_vec(),
            None => raw_segments,
        };
        let normalized_symbol = if symbol == "self" {
            String::new()
        } else {
            symbol
        };

        if let Some(reexport_vis) = ctx.reexport_visibility
            && let Some(src_mod) = resolved_module.clone()
        {
            if kind == ImportKind::Wildcard {
                self.exports.push(ExportDecl {
                    source_module: Some(src_mod),
                    symbol: String::new(),
                    exported_name: String::new(),
                    is_default: false,
                    is_wildcard: true,
                    visibility: reexport_vis,
                    origin: ExportOrigin::DirectInternal,
                    span: ctx.span.clone(),
                });
            } else if !normalized_symbol.is_empty() {
                let exported_name = alias.clone().unwrap_or_else(|| normalized_symbol.clone());
                self.exports.push(ExportDecl {
                    source_module: Some(src_mod),
                    symbol: normalized_symbol.clone(),
                    exported_name,
                    is_default: false,
                    is_wildcard: false,
                    visibility: reexport_vis,
                    origin: ExportOrigin::DirectInternal,
                    span: ctx.span.clone(),
                });
            }
        }

        self.imports.push(ImportDecl {
            resolved_module,
            module_segments,
            kind,
            symbol: normalized_symbol,
            alias,
            in_test_module: ctx.in_test_module,
            bypasses_visibility: false,
            origin,
            span: ctx.span.clone(),
        });
    }

    fn collect_use_tree(
        &mut self,
        node: &TsNode<'a>,
        prefix: &mut Vec<String>,
        ctx: &UseContext<'_>,
    ) {
        match node.kind().as_ref() {
            "identifier" => {
                let symbol = node.text().into_owned();
                if symbol == "self" {
                    let mut segs = prefix.clone();
                    if let Some(mod_sym) = segs.pop() {
                        if segs.is_empty() && self.declared_modules.contains(&mod_sym) {
                            segs.push(String::from("self"));
                        }
                        self.record_import(segs, ImportKind::Named, mod_sym, None, ctx);
                    }
                } else {
                    self.record_import(prefix.clone(), ImportKind::Named, symbol, None, ctx);
                }
            }
            "scoped_identifier" => {
                let text = node.text();
                let mut segs = prefix.clone();
                segs.extend(text.split("::").filter(|s| !s.is_empty()).map(String::from));
                if let Some(symbol) = segs.pop() {
                    self.record_import(segs, ImportKind::Named, symbol, None, ctx);
                }
            }
            "use_as_clause" => {
                let alias = node.field("alias").map(|n| n.text().into_owned());
                if let Some(path) = node.field("path") {
                    let text = path.text();
                    let mut segs = prefix.clone();
                    segs.extend(text.split("::").filter(|s| !s.is_empty()).map(String::from));
                    if let Some(mut symbol) = segs.pop() {
                        if symbol == "self" {
                            let Some(parent_sym) = segs.pop() else { return };
                            symbol = parent_sym;
                        }
                        if segs.is_empty() && self.declared_modules.contains(&symbol) {
                            segs.push(String::from("self"));
                        }
                        self.record_import(segs, ImportKind::Named, symbol, alias, ctx);
                    }
                }
            }
            "use_wildcard" => {
                let text = node.text();
                let path_part = text.trim().trim_end_matches('*').trim_end_matches("::");
                let mut segs = prefix.clone();
                segs.extend(
                    path_part
                        .split("::")
                        .filter(|s| !s.is_empty())
                        .map(String::from),
                );
                if !(ctx.in_test_module && segs.len() == 1 && segs[0] == "super") {
                    self.record_import(segs, ImportKind::Wildcard, "*".to_string(), None, ctx);
                }
            }
            "scoped_use_list" => {
                let saved_len = prefix.len();
                if let Some(path) = node.field("path") {
                    let text = path.text();
                    prefix.extend(text.split("::").filter(|s| !s.is_empty()).map(String::from));
                }
                if let Some(list) = node.field("list") {
                    for child in list.children() {
                        if child.is_named() {
                            self.collect_use_tree(&child, prefix, ctx);
                        }
                    }
                }
                prefix.truncate(saved_len);
            }
            "use_list" => {
                for child in node.children() {
                    if child.is_named() {
                        self.collect_use_tree(&child, prefix, ctx);
                    }
                }
            }
            _ => {}
        }
    }

    fn extract_struct(
        &self,
        node: &TsNode<'a>,
        attrs: &[String],
        in_test_module: bool,
    ) -> Option<TypeDecl> {
        let name = node.field("name")?.text().into_owned();
        let visibility = syntax::extract_visibility(node);
        let annotations = syntax::extract_derives(attrs);
        let has_implicit_field_readers = syntax::has_implicit_field_readers(&annotations);
        let mut fields = Vec::new();

        if let Some(body) = node.field("body") {
            match body.kind().as_ref() {
                "field_declaration_list" => self.extract_named_fields(&body, &mut fields),
                "ordered_field_declaration_list" => self.extract_tuple_fields(&body, &mut fields),
                _ => {}
            }
        }

        Some(TypeDecl {
            name,
            kind: TypeKind::Struct,
            visibility,
            in_test_module,
            export_binding: ExportBinding::Standard,
            has_implicit_field_readers,
            is_immutable_record: false,
            subclass_policy: SubclassPolicy::Closed,
            base_types: Vec::new(),
            fields,
            annotations,
            span: self.tree.node_span(node),
        })
    }

    fn extract_named_fields(&self, body: &TsNode<'a>, fields: &mut Vec<FieldDecl>) {
        for child in body.children() {
            if child.kind() != "field_declaration" {
                continue;
            }
            let Some(field_name) = child.field("name").map(|n| n.text().into_owned()) else {
                continue;
            };
            let (field_type, field_access) = child
                .field("type")
                .map_or((None, ParamAccess::Unknown), |t| {
                    syntax::extract_base_type(&t)
                });
            fields.push(FieldDecl {
                name: field_name,
                type_name: field_type,
                visibility: syntax::extract_visibility(&child),
                explicit_visibility: syntax::has_explicit_visibility(&child),
                is_mut_ref: field_access.is_mutable(),
                conditional_guard: ConditionalGuard::None,
                span: self.tree.node_span(&child),
            });
        }
    }

    fn extract_tuple_fields(&self, body: &TsNode<'a>, fields: &mut Vec<FieldDecl>) {
        let mut pending_vis = Visibility::Private;
        let mut idx = 0usize;
        for child in body.children() {
            match child.kind().as_ref() {
                "visibility_modifier" => pending_vis = syntax::parse_visibility_modifier(&child),
                "attribute_item" | "line_comment" | "block_comment" => {}
                _ if !child.is_named() => {
                    if child.text() == "," {
                        pending_vis = Visibility::Private;
                    }
                }
                _ => {
                    let (field_type, field_access) = syntax::extract_base_type(&child);
                    fields.push(FieldDecl {
                        name: idx.to_string(),
                        type_name: field_type,
                        visibility: pending_vis,
                        explicit_visibility: pending_vis != Visibility::Private,
                        is_mut_ref: field_access.is_mutable(),
                        conditional_guard: ConditionalGuard::None,
                        span: self.tree.node_span(&child),
                    });
                    idx += 1;
                    pending_vis = Visibility::Private;
                }
            }
        }
    }

    fn extract_enum(
        &self,
        node: &TsNode<'a>,
        attrs: &[String],
        in_test_module: bool,
    ) -> Option<TypeDecl> {
        let name = node.field("name")?.text().into_owned();
        let annotations = syntax::extract_derives(attrs);
        let has_implicit_field_readers = syntax::has_implicit_field_readers(&annotations);
        Some(TypeDecl {
            name,
            kind: TypeKind::Enum,
            visibility: syntax::extract_visibility(node),
            in_test_module,
            export_binding: ExportBinding::Standard,
            has_implicit_field_readers,
            is_immutable_record: false,
            subclass_policy: SubclassPolicy::Closed,
            base_types: Vec::new(),
            fields: Vec::new(),
            annotations,
            span: self.tree.node_span(node),
        })
    }

    fn extract_trait(&mut self, node: &TsNode<'a>, attrs: &[String], in_test_module: bool) {
        let Some(name_node) = node.field("name") else {
            return;
        };
        let name = name_node.text().into_owned();
        let visibility = syntax::extract_visibility(node);
        self.types.push(TypeDecl {
            name: name.clone(),
            kind: TypeKind::Interface,
            visibility,
            in_test_module,
            export_binding: ExportBinding::Standard,
            has_implicit_field_readers: false,
            is_immutable_record: false,
            subclass_policy: SubclassPolicy::Closed,
            base_types: syntax::extract_trait_supertraits(node),
            fields: Vec::new(),
            annotations: syntax::extract_derives(attrs),
            span: self.tree.node_span(node),
        });
        if let Some(body) = node.field("body") {
            let trait_ref = TypeRef::Named(name.clone());
            for child in body.children() {
                if child.kind() == "function_item"
                    && let Some(mut func) = self.extract_function(
                        &child,
                        &[],
                        Some(name.clone()),
                        Some(trait_ref.clone()),
                        in_test_module,
                    )
                {
                    func.visibility = visibility;
                    self.functions.push(func);
                }
            }
        }
    }

    fn extract_impl(&mut self, node: &TsNode<'a>, in_test_module: bool) {
        let interface_name = node
            .field("trait")
            .and_then(|t| syntax::extract_base_type(&t).0)
            .map(|tr| syntax::normalize_rust_type_ref(&self.module_path, &self.declared_modules, tr, in_test_module));
        let is_interface_impl = node.field("trait").is_some();
        let owner_type = node
            .field("type")
            .and_then(|t| match syntax::extract_base_type(&t).0 {
                Some(TypeRef::Named(name)) => Some(name),
                _ => None,
            });
        let impl_target = node
            .field("type")
            .and_then(|t| syntax::extract_impl_target_type(&t))
            .map(|tr| syntax::normalize_rust_type_ref(&self.module_path, &self.declared_modules, tr, in_test_module));

        if let (Some(iface), Some(target_type)) = (&interface_name, impl_target) {
            self.interface_impls.push(InterfaceImpl {
                interface_name: iface.clone(),
                target_type,
                module_path: None,
                in_test_module,
                role: syntax::classify_interface_impl_role(iface),
                span: self.tree.node_span(node),
            });
        }

        let Some(body) = node.field("body") else {
            return;
        };

        let mut pending_attrs = Vec::new();
        let mut pending_has_cfg_test = false;
        let mut compact_attr = String::new();
        for child in body.children() {
            match child.kind().as_ref() {
                "attribute_item" => {
                    let text = child.text();
                    compact_attr.clear();
                    compact_attr.extend(text.chars().filter(|c| !c.is_whitespace()));
                    if compact_attr.contains("cfg(test)") {
                        pending_has_cfg_test = true;
                    }
                    pending_attrs.push(text.into_owned());
                }
                "line_comment" | "block_comment" => {}
                "function_item" => {
                    let iface = is_interface_impl.then(|| {
                        interface_name
                            .clone()
                            .unwrap_or_else(|| TypeRef::Named(String::new()))
                    });
                    if let Some(func) = self.extract_function(
                        &child,
                        &pending_attrs,
                        owner_type.clone(),
                        iface,
                        in_test_module || pending_has_cfg_test,
                    ) {
                        self.functions.push(func);
                    }
                    pending_attrs.clear();
                    pending_has_cfg_test = false;
                }
                _ => {
                    pending_attrs.clear();
                    pending_has_cfg_test = false;
                }
            }
        }
    }

    fn extract_function(
        &self,
        node: &TsNode<'a>,
        attrs: &[String],
        owner_type: Option<String>,
        interface_name: Option<TypeRef>,
        in_test_module: bool,
    ) -> Option<FunctionDecl> {
        let name = node.field("name")?.text().into_owned();
        let visibility = syntax::extract_visibility(node);
        let is_interface_impl = interface_name.is_some();
        let interface_name =
            interface_name.filter(|r| !matches!(r, TypeRef::Named(s) if s.is_empty()));

        let role = if syntax::has_test_attr(attrs) {
            FunctionRole::Test
        } else if in_test_module {
            FunctionRole::TestHelper
        } else if owner_type.is_none() && name == "main" {
            FunctionRole::Entrypoint
        } else {
            FunctionRole::Production
        };

        let (self_receiver, params) = syntax::extract_function_params(node);
        let return_type = node
            .field("return_type")
            .and_then(|t| syntax::extract_base_type(&t).0);

        let body = BodyWalker::new(self.tree).extract(node.field("body"));
        Some(body.into_function_decl(
            FunctionHeader {
                name,
                owner_type,
                is_interface_impl,
                interface_name,
                self_receiver,
                visibility,
                explicit_visibility: syntax::has_explicit_visibility(node),
                export_binding: ExportBinding::Standard,
                is_lifecycle: false,
                role,
                params,
                return_type,
            },
            self.tree.node_span(node),
        ))
    }
}
