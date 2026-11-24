use super::TsNode;
use super::normalize::TsNormalizeDialect;
use super::syntax;
use crate::ast::item::{CallKind, CallReceiver, CallSite, LocalBinding, TypeRef};
use crate::ast::{ModulePath, SyntaxTree};
use crate::diagnostic::Span;
use crate::lang::common::{self, BodyAccumulator, BodyMetadataWalker, MemberFieldSpec};
use std::collections::HashMap;

pub(super) use crate::lang::common::ExtractedBody;

pub(super) struct TsBodyWalker<'a, 'b> {
    tree: &'a SyntaxTree,
    namespace_imports: &'b HashMap<String, ModulePath>,
    acc: BodyAccumulator,
}

impl<'a> BodyMetadataWalker<'a> for TsBodyWalker<'a, '_> {
    fn walk_body(&mut self, body: &TsNode<'a>) {
        self.walk(body, false);
    }

    fn into_accumulator(self) -> BodyAccumulator {
        self.acc
    }
}

impl<'a, 'b> TsBodyWalker<'a, 'b> {
    pub(super) fn new(
        tree: &'a SyntaxTree,
        namespace_imports: &'b HashMap<String, ModulePath>,
    ) -> Self {
        Self {
            tree,
            namespace_imports,
            acc: BodyAccumulator::default(),
        }
    }

    pub(super) fn extract(self, body: Option<TsNode<'a>>) -> ExtractedBody {
        common::extract_body_with_dialect(self, TsNormalizeDialect, body)
    }

    fn walk(&mut self, node: &TsNode<'a>, in_write_target: bool) {
        match node.kind().as_ref() {
            "comment" | "string" => return,
            "assignment_expression" | "augmented_assignment_expression" => {
                if let Some(left) = node.field("left") {
                    self.walk(&left, true);
                }
                if let Some(right) = node.field("right") {
                    self.walk(&right, false);
                }
                return;
            }
            "update_expression" => {
                if let Some(arg) = node.field("argument") {
                    self.walk(&arg, true);
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
                        self.walk(&child, false);
                    }
                }
                return;
            }
            "member_expression" => {
                let spec = MemberFieldSpec {
                    target_field: "object",
                    member_field: "property",
                    ident_kind: "identifier",
                    is_transparent: |kind| {
                        matches!(
                            kind,
                            "parenthesized_expression"
                                | "non_null_expression"
                                | "as_expression"
                                | "satisfies_expression"
                        )
                    },
                };
                if let Some((raw_obj, recorded)) =
                    self.acc
                        .record_ident_field_access(self.tree, node, spec, in_write_target)
                {
                    // Chained access (e.g. `order.customer.name = x`): attribute the write to
                    // `order.customer` rather than attributing `Customer.name` to `order`.
                    self.walk(&raw_obj, in_write_target && !recorded);
                }
                return;
            }
            "new_expression" => self.record_new_expression(node),
            "variable_declarator" => self.record_local_binding(node),
            _ => {}
        }

        if node.is_leaf() {
            if node.kind().as_ref() == "identifier" {
                self.acc.push_referenced_ident(node.text().into_owned());
            }
            return;
        }

        let keep_write = in_write_target
            && matches!(
                node.kind().as_ref(),
                "parenthesized_expression"
                    | "non_null_expression"
                    | "as_expression"
                    | "satisfies_expression"
            );
        for child in node.children() {
            self.walk(&child, keep_write);
        }
    }

    /// Walk a call callee without recording the invoked method name as a field access.
    fn walk_call_callee(&mut self, node: &TsNode<'a>) {
        if node.kind() == "member_expression" {
            for child in node.children() {
                self.walk(&child, false);
            }
        } else {
            self.walk(node, false);
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
        if name_node.kind() != "identifier" {
            return;
        }
        let name = name_node.text().into_owned();
        let annotated = node
            .field("type")
            .and_then(|t| syntax::extract_type_ref(&t));
        if annotated.is_some() {
            self.acc.push_local_binding(LocalBinding {
                name,
                type_name: annotated,
                init_callee: None,
            });
            return;
        }
        if let Some(val) = node.field("value") {
            let (type_name, init_callee) = self.infer_expr_origin(&val);
            if type_name.is_some() || init_callee.is_some() {
                self.acc.push_local_binding(LocalBinding {
                    name,
                    type_name,
                    init_callee,
                });
            }
        }
    }

    fn record_call(&mut self, call_node: &TsNode<'a>) {
        let Some(func_node) = call_node.field("function") else {
            return;
        };
        let span = self.tree.node_span(call_node);

        match func_node.kind().as_ref() {
            "identifier" => {
                self.acc.push_call(CallSite {
                    kind: CallKind::Free,
                    callee: func_node.text().into_owned(),
                    receiver: None,
                    span,
                });
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
                self.record_member_call(&raw_obj, callee, span);
            }
            _ => {}
        }
    }

    fn record_member_call(&mut self, raw_obj: &TsNode<'a>, callee: String, span: Span) {
        let mut obj = raw_obj.clone();
        while matches!(
            obj.kind().as_ref(),
            "parenthesized_expression" | "non_null_expression" | "satisfies_expression"
        ) || (obj.kind() == "as_expression"
            && obj
                .children()
                .filter(TsNode::is_named)
                .nth(1)
                .and_then(|t| syntax::extract_type_ref(&t))
                .is_none())
        {
            let Some(inner) = obj.children().find(TsNode::is_named) else {
                break;
            };
            obj = inner;
        }

        match obj.kind().as_ref() {
            "this" => self.acc.push_call(CallSite {
                kind: CallKind::Method,
                callee,
                receiver: Some(CallReceiver::SelfInstance),
                span,
            }),
            "identifier" => self.record_ident_member_call(&obj, callee, span),
            "member_expression" => self.record_nested_member_call(&obj, callee, span),
            _ => {
                let receiver = if let (Some(ty), _) = self.infer_expr_origin(&obj) {
                    match ty {
                        TypeRef::SelfType => CallReceiver::SelfType,
                        TypeRef::Named(name) => CallReceiver::Named(name),
                    }
                } else {
                    CallReceiver::Named(obj.text().into_owned())
                };
                self.acc.push_call(CallSite {
                    kind: CallKind::Method,
                    callee,
                    receiver: Some(receiver),
                    span,
                });
            }
        }
    }

    fn record_ident_member_call(&mut self, obj: &TsNode<'a>, callee: String, span: Span) {
        let obj_name = obj.text().into_owned();
        if let Some(ns_mod) = self.namespace_imports.get(&obj_name) {
            self.acc.record_scoped_type_call(
                CallReceiver::Named(ns_mod.to_string()),
                None,
                callee,
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
                span,
            );
        } else {
            self.acc.push_call(CallSite {
                kind: CallKind::Method,
                callee,
                receiver: Some(CallReceiver::Named(obj_name)),
                span,
            });
        }
    }

    fn record_nested_member_call(&mut self, obj: &TsNode<'a>, callee: String, span: Span) {
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
                span,
            );
        } else {
            self.acc.push_call(CallSite {
                kind: CallKind::Method,
                callee,
                receiver: Some(CallReceiver::Named(obj.text().into_owned())),
                span,
            });
        }
    }

    fn infer_expr_origin(&self, node: &TsNode<'_>) -> (Option<TypeRef>, Option<String>) {
        match node.kind().as_ref() {
            "parenthesized_expression"
            | "await_expression"
            | "non_null_expression"
            | "satisfies_expression" => node
                .children()
                .find(TsNode::is_named)
                .map_or((None, None), |c| self.infer_expr_origin(&c)),
            "as_expression" => {
                let mut named = node.children().filter(TsNode::is_named);
                let expr = named.next();
                let ty = named.next().and_then(|t| syntax::extract_type_ref(&t));
                if ty.is_some() {
                    (ty, None)
                } else {
                    expr.map_or((None, None), |e| self.infer_expr_origin(&e))
                }
            }
            "new_expression" => {
                let ty = node
                    .field("constructor")
                    .and_then(|c| extract_ctor_type_ref(&c));
                (ty, None)
            }
            "call_expression" => {
                let Some(func_node) = node.field("function") else {
                    return (None, None);
                };
                match func_node.kind().as_ref() {
                    "identifier" => (None, Some(func_node.text().into_owned())),
                    "member_expression" => {
                        let Some(obj) = func_node.field("object") else {
                            return (None, None);
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
                                return (Some(TypeRef::Named(obj_text.into_owned())), None);
                            }
                            if !callee.is_empty()
                                && self.namespace_imports.contains_key(obj_text.as_ref())
                            {
                                return (None, Some(callee));
                            }
                            return (None, None);
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
                            return (Some(ty), None);
                        }
                        (None, None)
                    }
                    _ => (None, None),
                }
            }
            _ => (None, None),
        }
    }
}

fn extract_ctor_type_ref(ctor: &TsNode<'_>) -> Option<TypeRef> {
    match ctor.kind().as_ref() {
        "identifier" => Some(TypeRef::Named(ctor.text().into_owned())),
        "member_expression" => syntax::join_dotted_path(&ctor.text()).map(TypeRef::Named),
        _ => None,
    }
}
