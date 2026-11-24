use super::members::{MemberContext, SwiftTypeIndex, qualify_name, subscript_body_node};
use super::{SwiftConventions, SwiftNode, expr, syntax};
use crate::ast::item::{
    ExportDecl, FieldDecl, FunctionDecl, ImportDecl, TypeDecl, TypeKind, Visibility,
};
use crate::ast::{FileAst, FileRole, ModulePath, SyntaxTree};

#[derive(Default)]
struct ExtractedItems {
    imports: Vec<ImportDecl>,
    exports: Vec<ExportDecl>,
    types: Vec<TypeDecl>,
    functions: Vec<FunctionDecl>,
    top_level_idents: Vec<String>,
}

pub(super) struct SwiftExtractor<'a> {
    tree: &'a SyntaxTree,
    module_path: ModulePath,
    file_role: FileRole,
    conventions: &'a SwiftConventions,
    index: SwiftTypeIndex<'a>,
    out: ExtractedItems,
}

impl<'a> SwiftExtractor<'a> {
    pub(super) fn new(
        tree: &'a SyntaxTree,
        module_path: ModulePath,
        file_role: FileRole,
        conventions: &'a SwiftConventions,
    ) -> Self {
        Self {
            tree,
            module_path,
            file_role,
            conventions,
            index: SwiftTypeIndex::new(tree, conventions),
            out: ExtractedItems::default(),
        }
    }

    pub(super) fn extract_file(mut self) -> FileAst {
        let in_test_file = self.file_role.is_test_file();
        let root = self.tree.root();

        for child in root.children() {
            match child.kind().as_ref() {
                "import_declaration" => {
                    if let Some(parsed) =
                        syntax::parse_import_declaration(&child, self.tree, in_test_file)
                    {
                        self.out.imports.push(parsed.import);
                        if let Some(reexport) = parsed.reexport {
                            self.out.exports.push(reexport);
                        }
                    }
                }
                "class_declaration" => self.index.index_type_members(&child, None, in_test_file),
                "protocol_declaration" => self.index.index_protocol_members(&child, None),
                _ => {}
            }
        }

        for child in root.children() {
            if child.kind() != "import_declaration" {
                self.walk_top_level(&child, in_test_file);
            }
        }

        self.out.top_level_idents.sort_unstable();
        self.out.top_level_idents.dedup();

        FileAst {
            path: self.tree.path().to_path_buf(),
            relative_path: self.tree.relative_path().to_path_buf(),
            module_path: self.module_path,
            language: self.tree.language_id(),
            member_separator: self.tree.language().member_separator(),
            role: self.file_role,
            imports: self.out.imports,
            exports: self.out.exports,
            types: self.out.types,
            functions: self.out.functions,
            top_level_idents: self.out.top_level_idents,
            suppressions: self.tree.extract_suppressions(),
        }
    }

    fn walk_top_level(&mut self, node: &SwiftNode<'a>, in_test_context: bool) {
        match node.kind().as_ref() {
            "class_declaration" => {
                self.extract_class_like(node, None, Visibility::Internal, in_test_context);
            }
            "protocol_declaration" => {
                self.extract_protocol(node, None, Visibility::Internal, in_test_context);
            }
            "function_declaration" => {
                if let Some(func) =
                    self.index
                        .build_free_function(node, in_test_context, self.file_role)
                {
                    self.out.functions.push(func);
                }
            }
            "property_declaration" if !in_test_context => self.collect_property_expr_idents(node),
            "comment" | "multiline_comment" | "typealias_declaration" | "property_declaration" => {}
            _ if !in_test_context => {
                syntax::collect_leaf_idents(node, &mut self.out.top_level_idents);
            }
            _ => {}
        }
    }

    fn extract_class_like(
        &mut self,
        node: &SwiftNode<'a>,
        parent_path: Option<&str>,
        default_vis: Visibility,
        in_test_context: bool,
    ) {
        let Some(kind_node) = node.field("declaration_kind") else {
            return;
        };
        let kind = match kind_node.text().as_ref() {
            "extension" => {
                self.extract_extension(node, in_test_context);
                return;
            }
            "struct" => TypeKind::Struct,
            "class" | "actor" => TypeKind::Class,
            "enum" => TypeKind::Enum,
            _ => return,
        };
        self.extract_nominal_type(node, parent_path, kind, default_vis, in_test_context);
    }

    fn extract_nominal_type(
        &mut self,
        node: &SwiftNode<'a>,
        parent_path: Option<&str>,
        kind: TypeKind,
        default_vis: Visibility,
        in_test_context: bool,
    ) {
        let Some(raw_name) = node.field("name").map(|n| n.text().into_owned()) else {
            return;
        };
        let name = qualify_name(parent_path, &raw_name);
        let body = node.field("body");
        if let Some(allow_prefix) = self.index.classify_test_container(
            node,
            &name,
            kind == TypeKind::Class,
            in_test_context,
        ) {
            if let Some(body_node) = body {
                self.extract_test_container_members(&body_node, &name, allow_prefix);
            }
            return;
        }

        let mut annotations = Vec::new();
        syntax::collect_attributes(node, &mut annotations);
        let visibility = syntax::extract_visibility(node, default_vis);
        let member_default_vis = if visibility == Visibility::Private {
            Visibility::Private
        } else {
            Visibility::Internal
        };
        let ctx = MemberContext {
            owner_name: &name,
            owner_kind: Some(kind),
            is_interface_impl: false,
            default_vis: member_default_vis,
            in_test_context,
            has_main_attr: annotations.iter().any(|a| a == "main"),
        };

        let mut fields = Vec::new();
        if let Some(body_node) = body {
            self.extract_body_members(&body_node, ctx, Some(&mut fields));
        }

        self.out.types.push(TypeDecl {
            name,
            kind,
            visibility,
            in_test_module: in_test_context,
            is_default_export: false,
            fields,
            annotations,
            span: self.tree.node_span(node),
        });
    }

    fn extract_extension(&mut self, node: &SwiftNode<'a>, in_test_context: bool) {
        let Some(owner_name) = syntax::extract_extension_target_name(node) else {
            return;
        };
        if let Some(allow_prefix) = self
            .index
            .test_container_prefix(&owner_name, in_test_context)
        {
            if let Some(body_node) = node.field("body") {
                self.extract_test_container_members(&body_node, &owner_name, allow_prefix);
            }
            return;
        }
        let ctx = MemberContext {
            owner_name: &owner_name,
            owner_kind: None,
            is_interface_impl: syntax::has_inheritance_specifier(node),
            default_vis: syntax::extract_visibility(node, Visibility::Internal),
            in_test_context,
            has_main_attr: false,
        };
        if let Some(body_node) = node.field("body") {
            self.extract_body_members(&body_node, ctx, None);
        }
    }

    fn extract_body_members(
        &mut self,
        body: &SwiftNode<'a>,
        ctx: MemberContext<'_>,
        mut owner_fields: Option<&mut Vec<FieldDecl>>,
    ) {
        for member in body.children() {
            match member.kind().as_ref() {
                "property_declaration" => {
                    self.extract_member_property(&member, ctx, owner_fields.as_deref_mut());
                }
                "function_declaration" => {
                    if let Some(func) = self.index.build_method(&member, ctx) {
                        self.out.functions.push(func);
                    }
                }
                "init_declaration" => {
                    self.out
                        .functions
                        .push(self.index.build_lifecycle_method(&member, ctx, true));
                }
                "deinit_declaration" => {
                    self.out
                        .functions
                        .push(self.index.build_lifecycle_method(&member, ctx, false));
                }
                "subscript_declaration" => {
                    self.out
                        .functions
                        .push(self.index.build_subscript(&member, ctx));
                }
                "class_declaration" => {
                    self.extract_class_like(
                        &member,
                        Some(ctx.owner_name),
                        ctx.default_vis,
                        ctx.in_test_context,
                    );
                }
                "protocol_declaration" => {
                    self.extract_protocol(
                        &member,
                        Some(ctx.owner_name),
                        ctx.default_vis,
                        ctx.in_test_context,
                    );
                }
                _ => {}
            }
        }
    }

    fn extract_member_property(
        &mut self,
        node: &SwiftNode<'a>,
        ctx: MemberContext<'_>,
        mut owner_fields: Option<&mut Vec<FieldDecl>>,
    ) {
        let is_static = syntax::is_static_or_class_member(node);
        let visibility = syntax::extract_visibility(node, ctx.default_vis);

        for binding in syntax::extract_property_bindings(node) {
            if let Some(computed) = &binding.computed_value {
                if is_static {
                    if !ctx.in_test_context {
                        syntax::collect_leaf_idents(computed, &mut self.out.top_level_idents);
                    }
                } else if let Some(func) = self
                    .index
                    .build_computed_property(node, &binding, ctx, visibility)
                {
                    self.out.functions.push(func);
                }
                continue;
            }

            if binding.observer_block.is_some()
                && let Some(func) = self
                    .index
                    .build_property_observer(node, &binding, ctx, visibility, is_static)
            {
                self.out.functions.push(func);
            }

            if !ctx.in_test_context
                && let Some(val) = binding.value
            {
                syntax::collect_leaf_idents(&val, &mut self.out.top_level_idents);
            }
            if !is_static
                && let Some(fields) = owner_fields.as_deref_mut()
                && let Some(field_name) = expr::extract_pattern_name(&binding.pattern)
            {
                let prop_type = binding.type_annotation.as_ref().and_then(|t| {
                    syntax::extract_type_ref(t, &self.conventions.transparent_type_wrappers)
                });
                fields.push(FieldDecl {
                    name: field_name,
                    type_name: prop_type,
                    visibility,
                    is_mut_ref: false,
                    span: self.tree.node_span(&binding.pattern),
                });
            }
        }
    }

    fn extract_test_container_members(
        &mut self,
        body: &SwiftNode<'a>,
        container_name: &str,
        allow_xctest_prefix: bool,
    ) {
        let mut param_names = Vec::new();
        for member in body.children() {
            match member.kind().as_ref() {
                "function_declaration" => {
                    if let Some(func) = self.index.build_test_function(
                        &member,
                        allow_xctest_prefix,
                        &mut param_names,
                    ) {
                        self.out.functions.push(func);
                    }
                }
                "init_declaration" => {
                    let params = syntax::extract_parameters(
                        &member,
                        &self.conventions.transparent_type_wrappers,
                    );
                    self.out.functions.push(self.index.build_test_helper_body(
                        &member,
                        String::from("init"),
                        params,
                        member.field("body"),
                    ));
                }
                "deinit_declaration" => {
                    self.out.functions.push(self.index.build_test_helper_body(
                        &member,
                        String::from("deinit"),
                        Vec::new(),
                        member.field("body"),
                    ));
                }
                "subscript_declaration" => {
                    let params = syntax::extract_parameters(
                        &member,
                        &self.conventions.transparent_type_wrappers,
                    );
                    self.out.functions.push(self.index.build_test_helper_body(
                        &member,
                        String::from("subscript"),
                        params,
                        subscript_body_node(&member),
                    ));
                }
                "property_declaration" => {
                    for binding in syntax::extract_property_bindings(&member) {
                        let name = expr::extract_pattern_name(&binding.pattern)
                            .unwrap_or_else(|| String::from("_"));
                        for body_node in [
                            binding.value,
                            binding.computed_value,
                            binding.observer_block,
                        ]
                        .into_iter()
                        .flatten()
                        {
                            self.out.functions.push(self.index.build_test_helper_body(
                                &member,
                                name.clone(),
                                Vec::new(),
                                Some(body_node),
                            ));
                        }
                    }
                }
                "class_declaration" => {
                    self.extract_class_like(
                        &member,
                        Some(container_name),
                        Visibility::Private,
                        true,
                    );
                }
                "protocol_declaration" => {
                    self.extract_protocol(&member, Some(container_name), Visibility::Private, true);
                }
                _ => {}
            }
        }
    }

    fn extract_protocol(
        &mut self,
        node: &SwiftNode<'a>,
        parent_path: Option<&str>,
        default_vis: Visibility,
        in_test_context: bool,
    ) {
        let Some(raw_name) = node.field("name").map(|n| n.text().into_owned()) else {
            return;
        };
        let name = qualify_name(parent_path, &raw_name);
        let mut annotations = Vec::new();
        syntax::collect_attributes(node, &mut annotations);
        let visibility = syntax::extract_visibility(node, default_vis);

        let fields = node
            .field("body")
            .map(|body| {
                body.children()
                    .filter(|m| m.kind() == "protocol_property_declaration")
                    .filter_map(|m| {
                        let field_name = expr::extract_pattern_name(&m.field("name")?)?;
                        let prop_type = m
                            .children()
                            .find(|c| c.kind() == "type_annotation")
                            .and_then(|t| {
                                syntax::extract_type_ref(
                                    &t,
                                    &self.conventions.transparent_type_wrappers,
                                )
                            });
                        Some(FieldDecl {
                            name: field_name,
                            type_name: prop_type,
                            visibility,
                            is_mut_ref: false,
                            span: self.tree.node_span(&m),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();

        self.out.types.push(TypeDecl {
            name,
            kind: TypeKind::Interface,
            visibility,
            in_test_module: in_test_context,
            is_default_export: false,
            fields,
            annotations,
            span: self.tree.node_span(node),
        });
    }

    fn collect_property_expr_idents(&mut self, prop: &SwiftNode<'a>) {
        for binding in syntax::extract_property_bindings(prop) {
            for expr_node in [
                binding.value,
                binding.computed_value,
                binding.observer_block,
            ]
            .into_iter()
            .flatten()
            {
                syntax::collect_leaf_idents(&expr_node, &mut self.out.top_level_idents);
            }
        }
    }
}
