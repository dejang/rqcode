use super::body::{SwiftBodyContext, SwiftBodyWalker};
use super::{SwiftConventions, SwiftNode, expr, syntax};
use crate::ast::item::{
    FunctionDecl, FunctionRole, ParamAccess, ParamDecl, TypeKind, TypeRef, Visibility,
};
use crate::ast::{FileRole, SyntaxTree};
use crate::lang::common::FunctionHeader;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy)]
pub(super) struct MemberContext<'a> {
    pub(super) owner_name: &'a str,
    pub(super) owner_kind: Option<TypeKind>,
    pub(super) is_interface_impl: bool,
    pub(super) default_vis: Visibility,
    pub(super) in_test_context: bool,
    pub(super) has_main_attr: bool,
}

/// Pass-1 symbol index of a Swift file's nominal type methods, properties, and test containers,
/// plus construction of [`FunctionDecl`] items backed by that index.
pub(super) struct SwiftTypeIndex<'a> {
    tree: &'a SyntaxTree,
    conventions: &'a SwiftConventions,
    type_methods: HashMap<String, HashSet<String>>,
    type_properties: HashMap<String, HashSet<String>>,
    test_containers: HashMap<String, bool>,
}

impl<'a> SwiftTypeIndex<'a> {
    pub(super) fn new(tree: &'a SyntaxTree, conventions: &'a SwiftConventions) -> Self {
        Self {
            tree,
            conventions,
            type_methods: HashMap::new(),
            type_properties: HashMap::new(),
            test_containers: HashMap::new(),
        }
    }

    pub(super) fn index_protocol_members(
        &mut self,
        node: &SwiftNode<'a>,
        parent_path: Option<&str>,
    ) {
        let Some(owner_name) = node
            .field("name")
            .map(|n| qualify_name(parent_path, n.text().as_ref()))
        else {
            return;
        };
        let Some(body) = node.field("body") else {
            return;
        };
        for member in body.children() {
            match member.kind().as_ref() {
                "protocol_function_declaration" => {
                    if let Some(name) = member.field("name").map(|n| n.text().into_owned()) {
                        self.type_methods
                            .entry(owner_name.clone())
                            .or_default()
                            .insert(name);
                    }
                }
                "protocol_property_declaration" if !syntax::is_static_or_class_member(&member) => {
                    if let Some(name_node) = member.field("name")
                        && let Some(prop_name) = expr::extract_pattern_name(&name_node)
                    {
                        self.type_properties
                            .entry(owner_name.clone())
                            .or_default()
                            .insert(prop_name);
                    }
                }
                _ => {}
            }
        }
    }

    pub(super) fn index_type_members(
        &mut self,
        node: &SwiftNode<'a>,
        parent_path: Option<&str>,
        in_test_context: bool,
    ) {
        let Some(kind_node) = node.field("declaration_kind") else {
            return;
        };
        let decl_kind = kind_node.text();
        let is_extension = decl_kind.as_ref() == "extension";
        let owner_name = if is_extension {
            syntax::extract_extension_target_name(node)
        } else {
            node.field("name")
                .map(|n| qualify_name(parent_path, n.text().as_ref()))
        };
        let Some(owner_name) = owner_name else {
            return;
        };

        let is_class = matches!(decl_kind.as_ref(), "class" | "actor");
        if !is_extension
            && let Some(allow_prefix) =
                self.classify_test_container(node, &owner_name, is_class, in_test_context)
        {
            self.test_containers
                .insert(owner_name.clone(), allow_prefix);
        }

        let Some(body) = node.field("body") else {
            return;
        };

        for member in body.children() {
            match member.kind().as_ref() {
                "function_declaration" => {
                    if let Some(name) = member.field("name").map(|n| n.text().into_owned()) {
                        self.type_methods
                            .entry(owner_name.clone())
                            .or_default()
                            .insert(name);
                    }
                }
                "init_declaration" => {
                    self.type_methods
                        .entry(owner_name.clone())
                        .or_default()
                        .insert(String::from("init"));
                }
                "property_declaration" if !syntax::is_static_or_class_member(&member) => {
                    for binding in syntax::extract_property_bindings(&member) {
                        if let Some(prop_name) = expr::extract_pattern_name(&binding.pattern) {
                            self.type_properties
                                .entry(owner_name.clone())
                                .or_default()
                                .insert(prop_name);
                        }
                    }
                }
                "class_declaration" => {
                    self.index_type_members(&member, Some(&owner_name), in_test_context);
                }
                "protocol_declaration" => {
                    self.index_protocol_members(&member, Some(&owner_name));
                }
                _ => {}
            }
        }
    }

    pub(super) fn classify_test_container(
        &self,
        node: &SwiftNode<'a>,
        qualified_name: &str,
        is_class: bool,
        in_test_context: bool,
    ) -> Option<bool> {
        let simple = qualified_name.rsplit("::").next().unwrap_or(qualified_name);
        let inherits_xctest = syntax::inherits_any(node, &self.conventions.test_base_classes);
        let is_suite = syntax::has_any_attribute(node, &self.conventions.test_suite_attributes);
        let has_test_name = self
            .conventions
            .test_container_name_suffixes
            .iter()
            .any(|suffix| simple.ends_with(suffix));
        let has_test_methods = node.field("body").is_some_and(|b| {
            b.children().any(|member| {
                member.kind() == "function_declaration"
                    && syntax::has_any_attribute(&member, &self.conventions.test_method_attributes)
            })
        });

        if inherits_xctest || is_suite || (in_test_context && (has_test_name || has_test_methods)) {
            // XCTest's `func test*()` convention applies to XCTestCase subclasses and
            // suffix-named test classes that are not Swift Testing `@Suite` containers.
            let allow_xctest_prefix = is_class && (inherits_xctest || (!is_suite && has_test_name));
            Some(allow_xctest_prefix)
        } else {
            None
        }
    }

    pub(super) fn test_container_prefix(
        &self,
        owner_name: &str,
        in_test_context: bool,
    ) -> Option<bool> {
        if let Some(&allow_prefix) = self.test_containers.get(owner_name) {
            return Some(allow_prefix);
        }
        if in_test_context {
            let simple = owner_name.rsplit("::").next().unwrap_or(owner_name);
            if self
                .conventions
                .test_container_name_suffixes
                .iter()
                .any(|suffix| simple.ends_with(suffix))
            {
                return Some(true);
            }
        }
        None
    }

    pub(super) fn build_computed_property(
        &self,
        node: &SwiftNode<'a>,
        binding: &syntax::SwiftPropertyBinding<'a>,
        ctx: MemberContext<'_>,
        visibility: Visibility,
    ) -> Option<FunctionDecl> {
        let computed = binding.computed_value.clone()?;
        self.build_property_accessor(node, binding, computed, ctx, (visibility, false, Some(ParamAccess::SharedRef)))
    }

    pub(super) fn build_property_observer(
        &self,
        node: &SwiftNode<'a>,
        binding: &syntax::SwiftPropertyBinding<'a>,
        ctx: MemberContext<'_>,
        visibility: Visibility,
        is_static: bool,
    ) -> Option<FunctionDecl> {
        let observer = binding.observer_block.clone()?;
        let self_recv = (!is_static).then_some(ParamAccess::MutableRef);
        self.build_property_accessor(node, binding, observer, ctx, (visibility, is_static, self_recv))
    }

    fn build_property_accessor(
        &self,
        node: &SwiftNode<'a>,
        binding: &syntax::SwiftPropertyBinding<'a>,
        body_node: SwiftNode<'a>,
        ctx: MemberContext<'_>,
        access: (Visibility, bool, Option<ParamAccess>),
    ) -> Option<FunctionDecl> {
        let (visibility, is_static, self_receiver) = access;
        let name = expr::extract_pattern_name(&binding.pattern)?;
        let prop_type = binding
            .type_annotation
            .as_ref()
            .and_then(|t| syntax::extract_type_ref(t, &self.conventions.transparent_type_wrappers));
        let body = SwiftBodyWalker::new(
            self.tree,
            self.body_context(Some(ctx.owner_name), is_static, &[]),
        )
        .extract(Some(body_node));
        Some(body.into_function_decl(
            FunctionHeader {
                name,
                owner_type: Some(ctx.owner_name.to_string()),
                is_interface_impl: ctx.is_interface_impl,
                interface_name: None,
                self_receiver,
                visibility,
                explicit_visibility: syntax::extract_visibility_explicit(node, ctx.default_vis).1,
                is_default_export: false,
                role: member_role(ctx.in_test_context),
                params: Vec::new(),
                return_type: prop_type,
            },
            self.tree.node_span(node),
        ))
    }

    pub(super) fn build_subscript(
        &self,
        node: &SwiftNode<'a>,
        ctx: MemberContext<'_>,
    ) -> FunctionDecl {
        let is_static = syntax::is_static_or_class_member(node);
        let params = syntax::extract_parameters(node, &self.conventions.transparent_type_wrappers);
        let param_names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
        let return_type = syntax::return_type_node(node).and_then(|t| {
            syntax::extract_type_ref(&t, &self.conventions.transparent_type_wrappers)
        });
        let body_node = subscript_body_node(node);
        let has_setter = body_node
            .as_ref()
            .is_some_and(|b| b.children().any(|c| c.kind() == "computed_setter"));
        let self_receiver = if is_static {
            None
        } else if has_setter || ctx.owner_kind == Some(TypeKind::Class) {
            Some(ParamAccess::MutableRef)
        } else {
            Some(ParamAccess::SharedRef)
        };
        let body = SwiftBodyWalker::new(
            self.tree,
            self.body_context(Some(ctx.owner_name), is_static, &param_names),
        )
        .extract(body_node);
        body.into_function_decl(
            FunctionHeader {
                name: String::from("subscript"),
                owner_type: Some(ctx.owner_name.to_string()),
                is_interface_impl: ctx.is_interface_impl,
                interface_name: None,
                self_receiver,
                visibility: syntax::extract_visibility(node, ctx.default_vis),
                explicit_visibility: syntax::extract_visibility_explicit(node, ctx.default_vis).1,
                is_default_export: false,
                role: member_role(ctx.in_test_context),
                params,
                return_type,
            },
            self.tree.node_span(node),
        )
    }

    pub(super) fn build_method(
        &self,
        node: &SwiftNode<'a>,
        ctx: MemberContext<'_>,
    ) -> Option<FunctionDecl> {
        let name = node.field("name")?.text().into_owned();
        let is_static = syntax::is_static_or_class_member(node);
        let self_receiver = if is_static {
            None
        } else if syntax::is_mutating(node) || ctx.owner_kind == Some(TypeKind::Class) {
            Some(ParamAccess::MutableRef)
        } else {
            Some(ParamAccess::SharedRef)
        };
        let return_type = syntax::return_type_node(node).and_then(|t| {
            syntax::extract_type_ref(&t, &self.conventions.transparent_type_wrappers)
        });
        let params = syntax::extract_parameters(node, &self.conventions.transparent_type_wrappers);
        let param_names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
        let role = if syntax::has_any_attribute(node, &self.conventions.test_method_attributes) {
            FunctionRole::Test
        } else if ctx.in_test_context {
            FunctionRole::TestHelper
        } else if ctx.has_main_attr && is_static && name == "main" {
            FunctionRole::Entrypoint
        } else {
            FunctionRole::Production
        };

        let body = SwiftBodyWalker::new(
            self.tree,
            self.body_context(Some(ctx.owner_name), is_static, &param_names),
        )
        .extract(node.field("body"));

        Some(body.into_function_decl(
            FunctionHeader {
                name,
                owner_type: Some(ctx.owner_name.to_string()),
                is_interface_impl: ctx.is_interface_impl,
                interface_name: None,
                self_receiver,
                visibility: syntax::extract_visibility(node, ctx.default_vis),
                explicit_visibility: syntax::extract_visibility_explicit(node, ctx.default_vis).1,
                is_default_export: false,
                role,
                params,
                return_type,
            },
            self.tree.node_span(node),
        ))
    }

    pub(super) fn build_lifecycle_method(
        &self,
        node: &SwiftNode<'a>,
        ctx: MemberContext<'_>,
        is_init: bool,
    ) -> FunctionDecl {
        let (name, self_receiver, visibility, params, return_type) = if is_init {
            (
                String::from("init"),
                None,
                syntax::extract_visibility(node, ctx.default_vis),
                syntax::extract_parameters(node, &self.conventions.transparent_type_wrappers),
                Some(TypeRef::SelfType),
            )
        } else {
            (
                String::from("deinit"),
                Some(ParamAccess::MutableRef),
                ctx.default_vis,
                Vec::new(),
                None,
            )
        };
        let param_names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
        let body = SwiftBodyWalker::new(
            self.tree,
            self.body_context(Some(ctx.owner_name), false, &param_names),
        )
        .extract(node.field("body"));

        body.into_function_decl(
            FunctionHeader {
                name,
                owner_type: Some(ctx.owner_name.to_string()),
                is_interface_impl: ctx.is_interface_impl,
                interface_name: None,
                self_receiver,
                visibility,
                explicit_visibility: is_init && syntax::extract_visibility_explicit(node, ctx.default_vis).1,
                is_default_export: false,
                role: member_role(ctx.in_test_context),
                params,
                return_type,
            },
            self.tree.node_span(node),
        )
    }

    pub(super) fn build_test_function(
        &self,
        member: &SwiftNode<'a>,
        allow_xctest_prefix: bool,
        param_names: &mut Vec<String>,
    ) -> Option<FunctionDecl> {
        let name = member.field("name")?.text().into_owned();
        let prefix = self.conventions.xctest_method_prefix.as_str();
        let params =
            syntax::extract_parameters(member, &self.conventions.transparent_type_wrappers);
        param_names.clear();
        param_names.extend(params.iter().map(|p| p.name.clone()));
        let is_static = syntax::is_static_or_class_member(member);
        let is_test = syntax::has_any_attribute(member, &self.conventions.test_method_attributes)
            || (allow_xctest_prefix
                && !is_static
                && params.is_empty()
                && syntax::is_void_return_type(member)
                && !prefix.is_empty()
                && name.starts_with(prefix)
                && name.len() > prefix.len());
        let role = if is_test {
            FunctionRole::Test
        } else {
            FunctionRole::TestHelper
        };
        let return_type = syntax::return_type_node(member).and_then(|t| {
            syntax::extract_type_ref(&t, &self.conventions.transparent_type_wrappers)
        });
        let extracted =
            SwiftBodyWalker::new(self.tree, self.body_context(None, false, param_names))
                .extract(member.field("body"));
        Some(extracted.into_function_decl(
            FunctionHeader {
                name,
                owner_type: None,
                is_interface_impl: false,
                interface_name: None,
                self_receiver: None,
                visibility: Visibility::Private,
                explicit_visibility: false,
                is_default_export: false,
                role,
                params,
                return_type,
            },
            self.tree.node_span(member),
        ))
    }

    pub(super) fn build_test_helper_body(
        &self,
        span_node: &SwiftNode<'a>,
        name: String,
        params: Vec<ParamDecl>,
        body_node: Option<SwiftNode<'a>>,
    ) -> FunctionDecl {
        let param_names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
        let body = SwiftBodyWalker::new(self.tree, self.body_context(None, false, &param_names))
            .extract(body_node);
        body.into_function_decl(
            FunctionHeader {
                name,
                owner_type: None,
                is_interface_impl: false,
                interface_name: None,
                self_receiver: None,
                visibility: Visibility::Private,
                explicit_visibility: false,
                is_default_export: false,
                role: FunctionRole::TestHelper,
                params,
                return_type: None,
            },
            self.tree.node_span(span_node),
        )
    }

    pub(super) fn build_free_function(
        &self,
        node: &SwiftNode<'a>,
        in_test_context: bool,
        file_role: FileRole,
    ) -> Option<FunctionDecl> {
        let name = node.field("name")?.text().into_owned();
        let params = syntax::extract_parameters(node, &self.conventions.transparent_type_wrappers);
        let param_names: Vec<String> = params.iter().map(|p| p.name.clone()).collect();
        let return_type = syntax::return_type_node(node).and_then(|t| {
            syntax::extract_type_ref(&t, &self.conventions.transparent_type_wrappers)
        });
        let role = if syntax::has_any_attribute(node, &self.conventions.test_method_attributes) {
            FunctionRole::Test
        } else if in_test_context {
            FunctionRole::TestHelper
        } else if file_role == FileRole::BinaryEntrypoint && name == "main" {
            FunctionRole::Entrypoint
        } else {
            FunctionRole::Production
        };
        let body = SwiftBodyWalker::new(self.tree, self.body_context(None, false, &param_names))
            .extract(node.field("body"));

        Some(body.into_function_decl(
            FunctionHeader {
                name,
                owner_type: None,
                is_interface_impl: false,
                interface_name: None,
                self_receiver: None,
                visibility: syntax::extract_visibility(node, Visibility::Internal),
                explicit_visibility: syntax::extract_visibility_explicit(node, Visibility::Internal).1,
                is_default_export: false,
                role,
                params,
                return_type,
            },
            self.tree.node_span(node),
        ))
    }

    fn body_context<'b>(
        &'b self,
        owner_name: Option<&str>,
        is_static_method: bool,
        param_names: &'b [String],
    ) -> SwiftBodyContext<'b> {
        let owner_methods = owner_name.and_then(|name| {
            self.type_methods.get(name).or_else(|| {
                let simple = name.rsplit("::").next().unwrap_or(name);
                self.type_methods.get(simple)
            })
        });
        let owner_properties = owner_name.and_then(|name| {
            self.type_properties.get(name).or_else(|| {
                let simple = name.rsplit("::").next().unwrap_or(name);
                self.type_properties.get(simple)
            })
        });
        SwiftBodyContext {
            owner_methods,
            owner_properties,
            transparent_wrappers: &self.conventions.transparent_type_wrappers,
            is_static_method,
            param_names,
        }
    }
}

pub(super) fn subscript_body_node<'a>(node: &SwiftNode<'a>) -> Option<SwiftNode<'a>> {
    node.field("body").or_else(|| {
        node.children()
            .find(|c| matches!(c.kind().as_ref(), "computed_property" | "statements"))
    })
}

pub(super) fn qualify_name(parent_path: Option<&str>, name: &str) -> String {
    match parent_path {
        Some(parent) if !parent.is_empty() => format!("{parent}::{name}"),
        _ => name.to_string(),
    }
}

fn member_role(in_test_context: bool) -> FunctionRole {
    if in_test_context {
        FunctionRole::TestHelper
    } else {
        FunctionRole::Production
    }
}
