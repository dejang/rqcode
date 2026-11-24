use super::{SwiftNode, coupling};
use super::expr::{self, SwiftCondition, SwiftConditionalParts};
use super::normalize::SwiftNormalizeDialect;
use super::syntax;
use crate::ast::SyntaxTree;
use crate::ast::item::{CallKind, CallReceiver, CallSite, FieldAccess, LocalBinding, TypeRef};
use crate::diagnostic::Span;
use crate::lang::common::{self, AccessMode, BodyAccumulator, BodyMetadataWalker};
use std::collections::HashSet;

pub(super) use crate::lang::common::ExtractedBody;

#[derive(Clone, Copy)]
pub(super) struct SwiftBodyContext<'b> {
    pub(super) owner_methods: Option<&'b HashSet<String>>,
    pub(super) owner_properties: Option<&'b HashSet<String>>,
    pub(super) transparent_wrappers: &'b [String],
    pub(super) is_static_method: bool,
    pub(super) param_names: &'b [String],
}

pub(super) struct SwiftBodyWalker<'a, 'b> {
    tree: &'a SyntaxTree,
    ctx: SwiftBodyContext<'b>,
    shadowed: HashSet<String>,
    acc: BodyAccumulator,
}

impl<'a> BodyMetadataWalker<'a> for SwiftBodyWalker<'a, '_> {
    fn walk_body(&mut self, body: &SwiftNode<'a>) {
        self.visit(body, AccessMode::Read);
    }

    fn into_accumulator(self) -> BodyAccumulator {
        self.acc
    }
}

impl<'a, 'b> SwiftBodyWalker<'a, 'b> {
    pub(super) fn new(tree: &'a SyntaxTree, ctx: SwiftBodyContext<'b>) -> Self {
        let shadowed = ctx.param_names.iter().cloned().collect();
        Self {
            tree,
            ctx,
            shadowed,
            acc: BodyAccumulator::default(),
        }
    }

    pub(super) fn extract(self, body: Option<SwiftNode<'a>>) -> ExtractedBody {
        let dialect = SwiftNormalizeDialect::new(
            self.ctx.owner_methods,
            self.ctx.owner_properties,
            self.ctx.is_static_method,
            self.ctx.param_names,
        );
        common::extract_body_with_dialect(self, dialect, body)
    }

    fn is_implicit_self_property(&self, name: &str) -> bool {
        !self.ctx.is_static_method
            && self.ctx.owner_properties.is_some_and(|p| p.contains(name))
            && !self.shadowed.contains(name)
    }

    fn visit_children(&mut self, node: &SwiftNode<'a>, mode: AccessMode) {
        for child in node.children() {
            self.visit(&child, mode);
        }
    }

    fn visit_labeled_children(&mut self, node: &SwiftNode<'a>, mode: AccessMode) {
        let children: Vec<SwiftNode<'a>> = node.children().collect();
        let mut i = 0;
        while i < children.len() {
            if children[i].kind() == "simple_identifier"
                && children
                    .get(i + 1)
                    .is_some_and(|n| !n.is_named() && n.text().trim() == ":")
            {
                i += 2;
                continue;
            }
            self.visit(&children[i], mode);
            i += 1;
        }
    }

    fn visit_scoped(&mut self, node: &SwiftNode<'a>, extra_bindings: Vec<String>) {
        let saved = self.shadowed.clone();
        self.shadowed.extend(extra_bindings);
        self.visit_children(node, AccessMode::Read);
        self.shadowed = saved;
    }

    fn visit_pattern_slice(&mut self, nodes: &[SwiftNode<'a>]) {
        let mut i = 0;
        while i < nodes.len() {
            let child = &nodes[i];
            if !child.is_named() && child.text().trim() == "." {
                i += 2;
                continue;
            }
            if child.kind() == "simple_identifier"
                && nodes
                    .get(i + 1)
                    .is_some_and(|n| !n.is_named() && n.text().trim() == ":")
            {
                i += 2;
                continue;
            }
            if !matches!(
                child.kind().as_ref(),
                "user_type" | "value_binding_pattern" | "type_annotation" | "async"
            ) {
                self.visit(child, AccessMode::Read);
            }
            i += 1;
        }
    }

    fn visit_conditional(&mut self, node: &SwiftNode<'a>) {
        let parts = SwiftConditionalParts::parse(node);
        let saved = self.shadowed.clone();
        for cond in parts.conditions() {
            match cond {
                SwiftCondition::OptionalBinding {
                    pattern,
                    bound_names,
                    rhs,
                } => {
                    if rhs.is_empty() {
                        self.visit_pattern_slice(pattern);
                    } else {
                        for item in rhs {
                            self.visit(item, AccessMode::Read);
                        }
                    }
                    self.shadowed.extend(bound_names.iter().cloned());
                }
                SwiftCondition::CaseBinding {
                    pattern,
                    bound_names,
                    rhs,
                } => {
                    for r in rhs {
                        self.visit(r, AccessMode::Read);
                    }
                    self.shadowed.extend(bound_names.iter().cloned());
                    self.visit_pattern_slice(pattern);
                }
                SwiftCondition::Expr(exprs) => {
                    for e in exprs {
                        self.visit(e, AccessMode::Read);
                    }
                }
            }
        }
        if node.kind() == "guard_statement" {
            let after_guard = self.shadowed.clone();
            self.shadowed = saved;
            for child in parts.else_children() {
                self.visit(child, AccessMode::Read);
            }
            self.shadowed = after_guard;
        } else {
            for child in parts.then_children() {
                self.visit(child, AccessMode::Read);
            }
            self.shadowed = saved;
            for child in parts.else_children() {
                self.visit(child, AccessMode::Read);
            }
        }
    }

    fn visit_scoped_node(&mut self, node: &SwiftNode<'a>, mode: AccessMode) -> bool {
        match node.kind().as_ref() {
            "statements" => self.visit_scoped(node, Vec::new()),
            "if_statement" | "guard_statement" | "while_statement" => self.visit_conditional(node),
            "switch_entry" => self.visit_scoped(node, expr::extract_switch_entry_bindings(node)),
            "catch_block" => self.visit_scoped(node, expr::extract_catch_bindings(node)),
            "function_declaration" => {
                if let Some(name) = node.field("name") {
                    self.shadowed.insert(name.text().into_owned());
                }
                if let Some(body) = node.field("body") {
                    self.visit_scoped(&body, expr::extract_nested_func_params(node));
                }
            }
            "pattern" => {
                let children: Vec<SwiftNode<'a>> = node.children().collect();
                self.visit_pattern_slice(&children);
            }
            "for_statement" => {
                let mut vars = Vec::new();
                if let Some(item) = node.field("item") {
                    expr::collect_pattern_names(&item, &mut vars);
                }
                let coll_range = node.field("collection").map(|coll| {
                    self.visit(&coll, AccessMode::Read);
                    coll.range()
                });
                let saved = self.shadowed.clone();
                self.shadowed.extend(vars);
                for child in node.children() {
                    if coll_range != Some(child.range()) {
                        self.visit(&child, AccessMode::Read);
                    }
                }
                self.shadowed = saved;
            }
            "lambda_literal" => {
                let (capture_reads, mut vars) = expr::extract_closure_capture_bindings(node);
                for read in &capture_reads {
                    self.visit(read, AccessMode::Read);
                }
                for child in node
                    .children()
                    .filter(|c| c.kind() == "lambda_function_type")
                {
                    syntax::collect_leaf_idents(&child, &mut vars);
                }
                let saved = self.shadowed.clone();
                self.shadowed.extend(vars);
                for child in node.children() {
                    if !matches!(
                        child.kind().as_ref(),
                        "capture_list" | "lambda_function_type"
                    ) {
                        self.visit(&child, AccessMode::Read);
                    }
                }
                self.shadowed = saved;
            }
            "tuple_expression" if syntax::unwrap_parenthesized_tuple(node).is_none() => {
                self.visit_labeled_children(node, mode);
            }
            "call_suffix" | "constructor_suffix" => self.visit_labeled_children(node, AccessMode::Read),
            _ => return false,
        }
        true
    }

    fn visit(&mut self, node: &SwiftNode<'a>, mode: AccessMode) {
        if self.visit_scoped_node(node, mode) {
            return;
        }
        match node.kind().as_ref() {
            "comment"
            | "multiline_comment"
            | "navigation_suffix"
            | "value_argument_label"
            | "key_path_expression"
            | "class_declaration"
            | "protocol_declaration" => return,
            "line_string_literal" | "multi_line_string_literal" | "raw_string_literal" => {
                let interp = if node.kind() == "raw_string_literal" {
                    "raw_str_interpolation"
                } else {
                    "interpolated_expression"
                };
                for child in node.children().filter(|c| c.kind() == interp) {
                    self.visit(&child, AccessMode::Read);
                }
                return;
            }
            "assignment" => {
                let compound = node.field("operator").is_some_and(|op| op.text().trim() != "=");
                if let Some(target) = node.field("target") {
                    self.visit(&target, if compound { AccessMode::ReadWrite } else { AccessMode::Write });
                }
                if let Some(result) = node.field("result") {
                    self.visit(&result, AccessMode::Read);
                }
                return;
            }
            "prefix_expression" => {
                let op = node.field("operation").map(|o| o.text());
                if op.as_deref().is_some_and(|s| s.trim() == ".")
                    && node
                        .field("target")
                        .is_some_and(|t| t.kind() == "simple_identifier")
                {
                    return;
                }
                let next = if op.as_deref().is_some_and(|s| s.trim() == "&") { AccessMode::ReadWrite } else { mode };
                if let Some(target) = node.field("target") {
                    self.visit(&target, next);
                }
                return;
            }
            "call_expression" => { coupling::inspect_body_call(self.tree, node, self.acc.coupling_mut()); return self.visit_call(node, mode); }
            "constructor_expression" => {
                coupling::inspect_body_call(self.tree, node, self.acc.coupling_mut());
                if let Some(ty_node) = node.field("constructed_type")
                    && let Some(ty_ref) = syntax::extract_nominal_type_ref(&ty_node)
                {
                    self.acc.push_constructed_type(ty_ref);
                }
                return self.visit_children(node, AccessMode::Read);
            }
            "navigation_expression" => return self.visit_navigation(node, mode),
            "property_declaration" => return self.visit_property_declaration(node),
            _ => {}
        }

        if node.is_leaf() {
            if node.kind().as_ref() == "simple_identifier" {
                let name = node.text().into_owned();
                if self.is_implicit_self_property(name.as_str()) {
                    self.acc.push_field_access(FieldAccess {
                        receiver_param: String::from("self"),
                        receiver_is_self: true,
                        receiver_type: None,
                        field_name: name.clone(),
                        is_read: mode.is_read(),
                        is_write: mode.is_write(),
                        span: self.tree.node_span(node),
                    });
                }
                self.acc.push_referenced_ident(name);
            }
            return;
        }

        let keep = matches!(
            node.kind().as_ref(),
            "directly_assignable_expression" | "postfix_expression" | "try_expression" | "await_expression"
        ) || syntax::unwrap_parenthesized_tuple(node).is_some();
        let next_mode = if keep { mode } else { AccessMode::Read };
        self.visit_children(node, next_mode);
    }

    fn visit_property_declaration(&mut self, node: &SwiftNode<'a>) {
        for binding in syntax::extract_property_bindings(node) {
            if let Some(name) = expr::extract_pattern_name(&binding.pattern) {
                let (type_name, init_callee) = match binding
                    .type_annotation
                    .as_ref()
                    .and_then(|t| syntax::extract_type_ref(t, self.ctx.transparent_wrappers))
                {
                    Some(annotated) => (Some(annotated), None),
                    None => binding.value.as_ref().map_or((None, None), |val| {
                        expr::infer_expr_origin(val, self.ctx.transparent_wrappers)
                    }),
                };
                if type_name.is_some() || init_callee.is_some() {
                    self.acc.push_local_binding(LocalBinding {
                        name,
                        type_name,
                        init_receiver: None,
                        init_callee,
                    });
                }
            }
            for expr_node in [
                binding.value,
                binding.computed_value,
                binding.observer_block,
            ]
            .into_iter()
            .flatten()
            {
                self.visit(&expr_node, AccessMode::Read);
            }
            let mut bound = Vec::new();
            expr::collect_pattern_names(&binding.pattern, &mut bound);
            self.shadowed.extend(bound);
        }
    }

    fn visit_navigation(&mut self, node: &SwiftNode<'a>, mode: AccessMode) {
        if expr::is_key_path_chain(node) {
            return;
        }
        let Some(raw_target) = node.field("target") else {
            return;
        };
        let mut cur = raw_target.clone();
        loop {
            if let Some(inner) = syntax::unwrap_parenthesized_tuple(&cur) {
                cur = inner;
                continue;
            }
            if matches!(
                cur.kind().as_ref(),
                "postfix_expression" | "try_expression" | "await_expression"
            ) && let Some(inner) = expr::unwrap_expr_child(&cur)
            {
                cur = inner;
                continue;
            }
            break;
        }
        let chained = if mode.is_write() { AccessMode::ReadWrite } else { AccessMode::Read };
        let (target_info, next_mode) = match cur.kind().as_ref() {
            "simple_identifier" if self.is_implicit_self_property(cur.text().as_ref()) => {
                (Some((String::new(), false, None)), chained)
            }
            "simple_identifier" | "self_expression" => {
                let is_self = cur.kind() == "self_expression";
                let param = if is_self { String::from("self") } else { cur.text().into_owned() };
                let info = (!syntax::is_uppercase_ident(&param)).then_some((param, is_self, None));
                (info, AccessMode::Read)
            }
            _ => {
                let ty = expr::infer_expr_origin(&cur, self.ctx.transparent_wrappers).0;
                (Some((String::new(), false, ty)), chained)
            }
        };
        if let Some((receiver_param, receiver_is_self, receiver_type)) = target_info
            && let Some(field_name) = syntax::extract_navigation_member(node)
        {
            self.acc.push_field_access(FieldAccess {
                receiver_param,
                receiver_is_self,
                receiver_type,
                field_name,
                is_read: mode.is_read(),
                is_write: mode.is_write(),
                span: self.tree.node_span(node),
            });
        }
        self.visit(&raw_target, next_mode);
    }

    fn visit_call(&mut self, node: &SwiftNode<'a>, mode: AccessMode) {
        let Some(callee) = node.children().find(SwiftNode::is_named) else {
            return;
        };
        let callee_range = callee.range();
        if expr::is_subscript_call(node) {
            self.visit(&callee, mode);
            for child in node.children() {
                if child.range() != callee_range {
                    self.visit(&child, AccessMode::Read);
                }
            }
            return;
        }
        self.record_call(node, &callee);
        for child in node.children() {
            if child.range() == callee_range && child.kind() == "navigation_expression" {
                self.visit_children(&child, AccessMode::Read);
            } else {
                self.visit(&child, AccessMode::Read);
            }
        }
    }

    fn record_call(&mut self, call_node: &SwiftNode<'a>, raw_callee: &SwiftNode<'a>) {
        let callee = expr::unwrap_transparent_callee(raw_callee);
        let span = self.tree.node_span(call_node);
        let kind = callee.kind();
        if let Some(ty) = syntax::shorthand_collection_type(kind.as_ref()) {
            self.acc
                .push_constructed_type(TypeRef::Named(String::from(ty)));
            return;
        }
        match kind.as_ref() {
            "simple_identifier" => self.record_simple_call(&callee, span),
            "navigation_expression" => {
                if let Some(member) = syntax::extract_navigation_member(&callee)
                    && let Some(raw_target) = callee.field("target")
                {
                    self.record_navigation_call(&raw_target, member, span);
                }
            }
            _ => {}
        }
    }

    fn record_simple_call(&mut self, ident: &SwiftNode<'a>, span: Span) {
        let name = ident.text().into_owned();
        if name == "Self" {
            self.acc.record_scoped_type_call(
                CallReceiver::SelfType,
                Some(TypeRef::SelfType),
                String::from("init"),
                Vec::new(),
                span,
            );
        } else if syntax::is_uppercase_ident(&name) {
            self.acc.record_scoped_type_call(
                CallReceiver::Named(name.clone()),
                Some(TypeRef::Named(name)),
                String::from("init"),
                Vec::new(),
                span,
            );
        } else if (!self.shadowed.contains(name.as_str())
            && self
                .ctx
                .owner_methods
                .is_some_and(|methods| methods.contains(name.as_str())))
            || self.is_implicit_self_property(name.as_str())
        {
            let receiver = if self.ctx.is_static_method {
                CallReceiver::SelfType
            } else {
                CallReceiver::SelfInstance
            };
            self.acc.push_call(CallSite {
                kind: CallKind::Method,
                callee: name,
                receiver: Some(receiver),
                arg_receivers: Vec::new(),
                span,
            });
        } else {
            self.acc.push_call(CallSite {
                kind: CallKind::Free,
                callee: name,
                receiver: None,
                arg_receivers: Vec::new(),
                span,
            });
        }
    }

    fn record_type_member_call(&mut self, prefix: String, member: String, span: Span) {
        let (ty_name, callee) = if syntax::is_uppercase_ident(&member) {
            (format!("{prefix}::{member}"), String::from("init"))
        } else {
            (prefix, member)
        };
        self.acc.record_scoped_type_call(
            CallReceiver::Named(ty_name.clone()),
            Some(TypeRef::Named(ty_name)),
            callee,
            Vec::new(),
            span,
        );
    }

    fn record_navigation_call(&mut self, raw_target: &SwiftNode<'a>, member: String, span: Span) {
        let mut target = expr::unwrap_transparent_callee(raw_target);
        while target.kind() == "as_expression"
            && target
                .field("type")
                .and_then(|t| syntax::extract_type_ref(&t, self.ctx.transparent_wrappers))
                .is_none()
        {
            let Some(inner) = expr::unwrap_expr_child(&target) else {
                break;
            };
            target = expr::unwrap_transparent_callee(&inner);
        }

        let receiver = match target.kind().as_ref() {
            "key_path_expression" => return,
            "self_expression" => CallReceiver::SelfInstance,
            "simple_identifier" => {
                let obj_name = target.text().into_owned();
                if obj_name == "Self" {
                    self.acc.record_scoped_type_call(
                        CallReceiver::SelfType,
                        Some(TypeRef::SelfType),
                        member,
                        Vec::new(),
                        span,
                    );
                    return;
                }
                if syntax::is_uppercase_ident(&obj_name) {
                    self.record_type_member_call(obj_name, member, span);
                    return;
                }
                CallReceiver::Named(obj_name)
            }
            "navigation_expression" => {
                if let Some(qualified) = expr::extract_qualified_type_path(&target) {
                    self.record_type_member_call(qualified, member, span);
                    return;
                }
                CallReceiver::Named(target.text().into_owned())
            }
            _ => match expr::infer_expr_origin(&target, self.ctx.transparent_wrappers) {
                (Some(TypeRef::SelfType), _) => CallReceiver::SelfType,
                (Some(TypeRef::Named(ty_name)), _) => CallReceiver::Named(ty_name),
                _ => CallReceiver::Named(target.text().into_owned()),
            },
        };
        self.acc.push_call(CallSite {
            kind: CallKind::Method,
            callee: member,
            receiver: Some(receiver),
            arg_receivers: Vec::new(),
            span,
        });
    }
}
