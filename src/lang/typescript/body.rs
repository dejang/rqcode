use super::TsNode;
use super::coupling;
use super::normalize::TsNormalizeDialect;
use super::syntax;
use crate::ast::item::{CallKind, CallReceiver, CallSite, FieldAccess, LocalBinding, TypeRef};
use crate::ast::{ModulePath, SyntaxTree};
use crate::diagnostic::Span;
use crate::lang::common::{self, AccessMode, BodyAccumulator, BodyMetadataWalker};
use std::collections::HashMap;

pub(super) use crate::lang::common::ExtractedBody;

pub(super) struct TsBodyWalker<'a, 'b> {
    tree: &'a SyntaxTree,
    base_dir: &'b [String],
    namespace_imports: &'b HashMap<String, ModulePath>,
    acc: BodyAccumulator,
}

impl<'a> BodyMetadataWalker<'a> for TsBodyWalker<'a, '_> {
    fn walk_body(&mut self, body: &TsNode<'a>) {
        self.walk(body, AccessMode::Read);
    }

    fn into_accumulator(self) -> BodyAccumulator {
        self.acc
    }
}

impl<'a, 'b> TsBodyWalker<'a, 'b> {
    pub(super) fn new(
        tree: &'a SyntaxTree,
        base_dir: &'b [String],
        namespace_imports: &'b HashMap<String, ModulePath>,
    ) -> Self {
        Self {
            tree,
            base_dir,
            namespace_imports,
            acc: BodyAccumulator::default(),
        }
    }

    pub(super) fn extract(self, body: Option<TsNode<'a>>) -> ExtractedBody {
        common::extract_body_with_dialect(self, TsNormalizeDialect, body)
    }

    fn walk(&mut self, node: &TsNode<'a>, mode: AccessMode) {
        coupling::TsCouplingInspector::new(self.tree, self.base_dir, self.acc.coupling_mut())
            .inspect(node);
        match node.kind().as_ref() {
            "comment" | "string" => return,
            "assignment_expression" | "augmented_assignment_expression" => {
                let lhs_mode = if node.kind() == "augmented_assignment_expression" {
                    AccessMode::ReadWrite
                } else {
                    AccessMode::Write
                };
                if let Some(left) = node.field("left") {
                    self.walk(&left, lhs_mode);
                }
                if let Some(right) = node.field("right") {
                    self.walk(&right, AccessMode::Read);
                }
                return;
            }
            "update_expression" => {
                if let Some(arg) = node.field("argument") {
                    self.walk(&arg, AccessMode::ReadWrite);
                }
                return;
            }
            "call_expression" => {
                self.record_call(node);
                let func_range = node.field("function").map(|f| f.range());
                for child in node.children() {
                    if func_range == Some(child.range()) {
                        self.walk_call_callee(&child);
                    } else {
                        self.walk(&child, AccessMode::Read);
                    }
                }
                return;
            }
            "member_expression" => {
                if let Some(raw_obj) = node.field("object")
                    && let Some(prop) = node.field("property")
                {
                    self.record_member_or_subscript_access(
                        node,
                        &raw_obj,
                        prop.text().into_owned(),
                        mode,
                    );
                }
                return;
            }
            "subscript_expression" => {
                if let Some(index_node) = node.field("index")
                    && let Some(field_name) = extract_static_string_key(&index_node)
                    && let Some(raw_obj) = node.field("object")
                {
                    self.record_member_or_subscript_access(node, &raw_obj, field_name, mode);
                    return;
                }
            }
            "new_expression" => self.record_new_expression(node),
            "variable_declarator" => self.record_local_binding(node),
            "required_parameter" | "optional_parameter" => {
                if let Some(pat) = node.field("pattern")
                    && pat.kind() == "object_pattern"
                {
                    let ty = node
                        .field("type")
                        .and_then(|t| syntax::extract_type_ref(&t));
                    self.record_object_pattern_reads(&pat, "", false, ty.as_ref());
                }
            }
            _ => {}
        }

        if node.is_leaf() {
            if node.kind().as_ref() == "identifier" {
                self.acc.push_referenced_ident(node.text().into_owned());
            }
            return;
        }

        let next_mode = if is_ts_transparent_node(node) {
            mode
        } else {
            AccessMode::Read
        };
        for child in node.children() {
            self.walk(&child, next_mode);
        }
    }

    fn record_member_or_subscript_access(
        &mut self,
        node: &TsNode<'a>,
        raw_obj: &TsNode<'a>,
        field_name: String,
        mode: AccessMode,
    ) {
        let target = unwrap_ts_receiver(raw_obj);
        let (receiver_param, receiver_is_self, receiver_type, direct) = match target.kind().as_ref()
        {
            "this" => (String::new(), true, None, true),
            "identifier" => (target.text().into_owned(), false, None, true),
            _ => (
                String::new(),
                false,
                self.infer_expr_origin(&target).0,
                false,
            ),
        };
        self.acc.push_field_access(FieldAccess {
            receiver_param,
            receiver_is_self,
            receiver_type,
            field_name,
            is_read: mode.is_read(),
            is_write: mode.is_write(),
            span: self.tree.node_span(node),
        });
        let next = if mode.is_write() && !direct {
            AccessMode::ReadWrite
        } else {
            AccessMode::Read
        };
        self.walk(raw_obj, next);
    }

    /// Walk a call callee without recording the invoked method name as a field access.
    fn walk_call_callee(&mut self, node: &TsNode<'a>) {
        if matches!(
            node.kind().as_ref(),
            "member_expression" | "subscript_expression"
        ) {
            for child in node.children() {
                self.walk(&child, AccessMode::Read);
            }
        } else {
            self.walk(node, AccessMode::Read);
        }
    }

    fn record_new_expression(&mut self, node: &TsNode<'a>) {
        let Some(ctor) = node.field("constructor") else {
            return;
        };
        if let Some(ty_ref) = extract_ctor_type_ref(&ctor) {
            self.acc.push_constructed_type(ty_ref);
            if ctor.kind() == "member_expression"
                && let Some(prop) = ctor.field("property")
            {
                self.acc.push_scoped_ident_ref(prop.text().into_owned());
            }
        }
    }

    fn record_local_binding(&mut self, node: &TsNode<'a>) {
        let Some(name_node) = node.field("name") else {
            return;
        };
        if name_node.kind() == "object_pattern" {
            if let Some(val) = node.field("value") {
                self.record_object_destructuring(&name_node, &val);
            }
            return;
        }
        if name_node.kind() != "identifier" {
            return;
        }
        let name = name_node.text().into_owned();
        let annotated = node
            .field("type")
            .and_then(|t| syntax::extract_type_ref(&t));
        if annotated.is_some() {
            self.acc.push_local_binding(LocalBinding { name, type_name: annotated, init_receiver: None, init_callee: None });
            return;
        }
        if let Some(val) = node.field("value") {
            let (type_name, init_receiver, init_callee) = self.infer_expr_origin(&val);
            if type_name.is_some() || init_callee.is_some() {
                self.acc.push_local_binding(LocalBinding { name, type_name, init_receiver, init_callee });
            }
        }
    }

    fn record_object_destructuring(&mut self, pattern: &TsNode<'a>, val: &TsNode<'a>) {
        let unwrapped = unwrap_ts_receiver(val);
        let (receiver_param, receiver_is_self, receiver_type) = match unwrapped.kind().as_ref() {
            "this" => (String::new(), true, None),
            "identifier" => (unwrapped.text().into_owned(), false, None),
            _ => (String::new(), false, self.infer_expr_origin(&unwrapped).0),
        };
        self.record_object_pattern_reads(
            pattern,
            &receiver_param,
            receiver_is_self,
            receiver_type.as_ref(),
        );
    }

    fn record_object_pattern_reads(
        &mut self,
        pattern: &TsNode<'a>,
        receiver_param: &str,
        receiver_is_self: bool,
        receiver_type: Option<&TypeRef>,
    ) {
        for child in pattern.children() {
            let field_name = match child.kind().as_ref() {
                "shorthand_property_identifier_pattern" => Some(child.text().into_owned()),
                "pair_pattern" => child.field("key").and_then(|k| {
                    extract_static_string_key(&k).or_else(|| Some(k.text().into_owned()))
                }),
                _ => None,
            };
            if let Some(field_name) = field_name {
                self.acc.push_field_access(FieldAccess {
                    receiver_param: receiver_param.to_string(),
                    receiver_is_self,
                    receiver_type: receiver_type.cloned(),
                    field_name,
                    is_read: true,
                    is_write: false,
                    span: self.tree.node_span(&child),
                });
            }
        }
    }

    fn record_call(&mut self, call_node: &TsNode<'a>) {
        let Some(func_node) = call_node.field("function") else {
            return;
        };
        let span = self.tree.node_span(call_node);
        let arg_receivers = self.extract_arg_receivers(call_node);

        match func_node.kind().as_ref() {
            "identifier" => {
                self.acc.push_call(CallSite { kind: CallKind::Free, callee: func_node.text().into_owned(), receiver: None, arg_receivers, span });
            }
            "member_expression" => {
                let Some(callee) = func_node
                    .field("property")
                    .map(|p| p.text().into_owned())
                    .filter(|s| !s.is_empty())
                else {
                    return;
                };
                let Some(raw_obj) = func_node.field("object") else {
                    return;
                };
                self.record_member_call(&raw_obj, callee, arg_receivers, span);
            }
            "subscript_expression" => {
                let Some(index_node) = func_node.field("index") else {
                    return;
                };
                let Some(callee) = extract_static_string_key(&index_node) else {
                    return;
                };
                let Some(raw_obj) = func_node.field("object") else {
                    return;
                };
                self.record_member_call(&raw_obj, callee, arg_receivers, span);
            }
            _ => {}
        }
    }

    fn extract_arg_receivers(&self, call_node: &TsNode<'a>) -> Vec<Option<CallReceiver>> {
        let Some(args) = call_node.field("arguments") else {
            return Vec::new();
        };
        args.children()
            .filter(TsNode::is_named)
            .map(|arg| self.extract_expr_receiver(&arg))
            .collect()
    }

    fn extract_expr_receiver(&self, node: &TsNode<'a>) -> Option<CallReceiver> {
        let unwrapped = unwrap_ts_receiver(node);
        match unwrapped.kind().as_ref() {
            "this" => Some(CallReceiver::SelfInstance),
            "identifier" => Some(CallReceiver::Named(unwrapped.text().into_owned())),
            _ => match self.infer_expr_origin(&unwrapped).0 {
                Some(TypeRef::SelfType) => Some(CallReceiver::SelfType),
                Some(TypeRef::Named(name)) => Some(CallReceiver::Named(name)),
                None => None,
            },
        }
    }

    fn record_member_call(
        &mut self,
        raw_obj: &TsNode<'a>,
        callee: String,
        arg_receivers: Vec<Option<CallReceiver>>,
        span: Span,
    ) {
        let obj = unwrap_ts_receiver(raw_obj);

        match obj.kind().as_ref() {
            "this" => self.acc.push_call(CallSite {
                kind: CallKind::Method,
                callee,
                receiver: Some(CallReceiver::SelfInstance),
                arg_receivers,
                span,
            }),
            "identifier" => self.record_ident_member_call(&obj, callee, arg_receivers, span),
            "member_expression" => {
                self.record_nested_member_call(&obj, callee, arg_receivers, span);
            }
            _ => {
                let receiver = if let (Some(ty), _, _) = self.infer_expr_origin(&obj) {
                    match ty {
                        TypeRef::SelfType => CallReceiver::SelfType,
                        TypeRef::Named(name) => CallReceiver::Named(name),
                    }
                } else {
                    CallReceiver::Named(obj.text().into_owned())
                };
                self.acc.push_call(CallSite { kind: CallKind::Method, callee, receiver: Some(receiver), arg_receivers, span });
            }
        }
    }

    fn record_ident_member_call(
        &mut self,
        obj: &TsNode<'a>,
        callee: String,
        arg_receivers: Vec<Option<CallReceiver>>,
        span: Span,
    ) {
        let obj_name = obj.text().into_owned();
        if let Some(ns_mod) = self.namespace_imports.get(&obj_name) {
            self.acc.record_scoped_type_call(
                CallReceiver::Named(ns_mod.to_string()),
                None,
                callee,
                arg_receivers,
                span,
            );
        } else if obj_name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase())
        {
            self.acc.record_scoped_type_call(
                CallReceiver::Named(obj_name.clone()),
                Some(TypeRef::Named(obj_name)),
                callee,
                arg_receivers,
                span,
            );
        } else {
            self.acc.push_call(CallSite {
                kind: CallKind::Method,
                callee,
                receiver: Some(CallReceiver::Named(obj_name)),
                arg_receivers,
                span,
            });
        }
    }

    fn record_nested_member_call(
        &mut self,
        obj: &TsNode<'a>,
        callee: String,
        arg_receivers: Vec<Option<CallReceiver>>,
        span: Span,
    ) {
        if let Some(ns_node) = obj.field("object")
            && ns_node.kind() == "identifier"
            && let Some(ty_node) = obj.field("property")
            && ty_node
                .text()
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_uppercase())
        {
            let qualified = format!("{}::{}", ns_node.text(), ty_node.text());
            self.acc.record_scoped_type_call(
                CallReceiver::Named(qualified.clone()),
                Some(TypeRef::Named(qualified)),
                callee,
                arg_receivers,
                span,
            );
        } else {
            self.acc.push_call(CallSite {
                kind: CallKind::Method,
                callee,
                receiver: Some(CallReceiver::Named(obj.text().into_owned())),
                arg_receivers,
                span,
            });
        }
    }

    fn infer_expr_origin(
        &self,
        node: &TsNode<'_>,
    ) -> (Option<TypeRef>, Option<CallReceiver>, Option<String>) {
        match node.kind().as_ref() {
            "parenthesized_expression"
            | "await_expression"
            | "non_null_expression"
            | "satisfies_expression" => node
                .children()
                .find(TsNode::is_named)
                .map_or((None, None, None), |c| self.infer_expr_origin(&c)),
            "as_expression" => {
                let mut named = node.children().filter(TsNode::is_named);
                let expr = named.next();
                let ty = named.next().and_then(|t| syntax::extract_type_ref(&t));
                if ty.is_some() {
                    (ty, None, None)
                } else {
                    expr.map_or((None, None, None), |e| self.infer_expr_origin(&e))
                }
            }
            "new_expression" => {
                let ty = node
                    .field("constructor")
                    .and_then(|c| extract_ctor_type_ref(&c));
                (ty, None, None)
            }
            "call_expression" => {
                let Some(func_node) = node.field("function") else {
                    return (None, None, None);
                };
                match func_node.kind().as_ref() {
                    "identifier" => (None, None, Some(func_node.text().into_owned())),
                    "member_expression" => {
                        let Some(obj) = func_node.field("object") else {
                            return (None, None, None);
                        };
                        let callee = func_node
                            .field("property")
                            .map(|p| p.text().into_owned())
                            .unwrap_or_default();
                        if obj.kind() == "identifier" {
                            let obj_text = obj.text();
                            if obj_text
                                .chars()
                                .next()
                                .is_some_and(|c| c.is_ascii_uppercase())
                            {
                                return (Some(TypeRef::Named(obj_text.into_owned())), None, None);
                            }
                            if !callee.is_empty()
                                && self.namespace_imports.contains_key(obj_text.as_ref())
                            {
                                return (None, None, Some(callee));
                            }
                            if !callee.is_empty() {
                                return (
                                    None,
                                    Some(CallReceiver::Named(obj_text.into_owned())),
                                    Some(callee),
                                );
                            }
                            return (None, None, None);
                        }
                        if obj.kind() == "this" && !callee.is_empty() {
                            return (None, Some(CallReceiver::SelfInstance), Some(callee));
                        }
                        if obj.kind() == "member_expression"
                            && let Some(prop) = obj.field("property")
                            && prop
                                .text()
                                .chars()
                                .next()
                                .is_some_and(|c| c.is_ascii_uppercase())
                            && let Some(ty) = extract_ctor_type_ref(&obj)
                        {
                            return (Some(ty), None, None);
                        }
                        (None, None, None)
                    }
                    _ => (None, None, None),
                }
            }
            _ => (None, None, None),
        }
    }
}

fn is_ts_transparent_node(node: &TsNode<'_>) -> bool {
    matches!(
        node.kind().as_ref(),
        "parenthesized_expression"
            | "non_null_expression"
            | "satisfies_expression"
            | "await_expression"
    ) || (node.kind() == "as_expression"
        && node
            .children()
            .filter(TsNode::is_named)
            .nth(1)
            .and_then(|t| syntax::extract_type_ref(&t))
            .is_none())
}

fn unwrap_ts_receiver<'a>(node: &TsNode<'a>) -> TsNode<'a> {
    let mut cur = node.clone();
    while is_ts_transparent_node(&cur) {
        let Some(inner) = cur
            .children()
            .find(|c| c.is_named() && !common::is_comment_kind(c.kind().as_ref()))
        else {
            break;
        };
        cur = inner;
    }
    cur
}

fn extract_static_string_key(node: &TsNode<'_>) -> Option<String> {
    match node.kind().as_ref() {
        "string" => {
            let unquoted = syntax::unquote_string(&node.text());
            (!unquoted.is_empty()).then_some(unquoted)
        }
        "template_string" => {
            if node.children().any(|c| c.kind() == "template_substitution") {
                return None;
            }
            let unquoted = syntax::unquote_string(&node.text());
            (!unquoted.is_empty()).then_some(unquoted)
        }
        _ => None,
    }
}

fn extract_ctor_type_ref(ctor: &TsNode<'_>) -> Option<TypeRef> {
    match ctor.kind().as_ref() {
        "identifier" => Some(TypeRef::Named(ctor.text().into_owned())),
        "member_expression" => syntax::join_dotted_path(&ctor.text()).map(TypeRef::Named),
        _ => None,
    }
}
