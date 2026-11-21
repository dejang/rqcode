use super::TsNode;
use super::syntax;
use crate::ast::item::{CallKind, CallReceiver, CallSite, FieldAccess, LocalBinding, TypeRef};
use crate::ast::vector::NormToken;
use crate::ast::{ModulePath, SyntaxTree};
use crate::diagnostic::Span;
use crate::lang::common::{BodyAccumulator, LeafClass};
use std::collections::HashMap;

pub(super) use crate::lang::common::ExtractedBody;

pub(super) struct TsBodyWalker<'a, 'b> {
    tree: &'a SyntaxTree,
    namespace_imports: &'b HashMap<String, ModulePath>,
    acc: BodyAccumulator,
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

    pub(super) fn extract(mut self, body: Option<TsNode<'a>>) -> ExtractedBody {
        if let Some(body) = body {
            self.walk(&body, false);
        }
        self.acc.finish()
    }

    fn walk(&mut self, node: &TsNode<'a>, in_write_target: bool) {
        if self.handle_special_node(node) {
            return;
        }
        self.record_structural_token(node, in_write_target);

        if node.is_leaf() {
            let class = match node.kind().as_ref() {
                "identifier"
                | "property_identifier"
                | "private_property_identifier"
                | "shorthand_property_identifier"
                | "shorthand_property_identifier_pattern"
                | "type_identifier" => LeafClass::Ident,
                "number" | "string_fragment" | "regex_pattern" | "true" | "false" | "null"
                | "undefined" => LeafClass::Lit,
                "(" | ")" | "{" | "}" | "[" | "]" | ";" | "," | "." | ":" | "\"" | "'" | "`"
                | "?." | "const" | "let" | "var" => LeafClass::Skip,
                _ => LeafClass::Other,
            };
            self.acc.record_leaf(node, class);
            return;
        }

        for child in node.children() {
            self.walk(&child, false);
        }
    }

    /// Handle nodes with custom child traversal order; returns `true` when traversal is complete.
    fn handle_special_node(&mut self, node: &TsNode<'a>) -> bool {
        match node.kind().as_ref() {
            "comment" => true,
            "string" => {
                self.acc.push_token(NormToken::Lit);
                true
            }
            "assignment_expression" | "augmented_assignment_expression" => {
                self.walk_assignment(node);
                true
            }
            "update_expression" => {
                self.acc.push_token(NormToken::Assign);
                if let Some(op) = node.field("operator") {
                    self.acc
                        .push_token(NormToken::Other(op.text().into_owned()));
                }
                if let Some(arg) = node.field("argument") {
                    self.walk(&arg, true);
                }
                true
            }
            "call_expression" => {
                self.walk_call_expression(node);
                true
            }
            _ => false,
        }
    }

    fn record_structural_token(&mut self, node: &TsNode<'a>, in_write_target: bool) {
        let token = match node.kind().as_ref() {
            "member_expression" => {
                self.record_field_access(node, in_write_target);
                Some(NormToken::Field)
            }
            "new_expression" => {
                self.record_new_expression(node);
                Some(NormToken::Construct)
            }
            "variable_declarator" => {
                self.record_local_binding(node);
                Some(NormToken::Bind)
            }
            "if_statement" | "ternary_expression" => Some(NormToken::If),
            "switch_statement" => Some(NormToken::Branch),
            "switch_case" | "switch_default" => Some(NormToken::BranchArm),
            "for_statement" | "for_in_statement" | "while_statement" | "do_statement" => {
                Some(NormToken::Loop)
            }
            "return_statement" => Some(NormToken::Return),
            "arrow_function" | "function_expression" => Some(NormToken::Closure),
            "try_statement" => Some(NormToken::Try),
            _ => None,
        };
        if let Some(tok) = token {
            self.acc.push_token(tok);
        }
    }

    fn walk_assignment(&mut self, node: &TsNode<'a>) {
        self.acc.push_token(NormToken::Assign);
        if let Some(left) = node.field("left") {
            self.walk(&left, true);
        }
        if let Some(op) = node.field("operator") {
            self.acc
                .push_token(NormToken::Other(op.text().into_owned()));
        }
        if let Some(right) = node.field("right") {
            self.walk(&right, false);
        }
    }

    fn walk_call_expression(&mut self, node: &TsNode<'a>) {
        self.record_call(node);
        self.acc.push_token(NormToken::Call);
        let func_range = node.field("function").map(|f| f.range());
        for child in node.children() {
            if func_range == Some(child.range()) {
                self.walk_call_callee(&child);
            } else {
                self.walk(&child, false);
            }
        }
    }

    /// Walk a call callee without recording the invoked method name as a field access.
    fn walk_call_callee(&mut self, node: &TsNode<'a>) {
        if node.kind() == "member_expression" {
            self.acc.push_token(NormToken::Field);
            for child in node.children() {
                self.walk(&child, false);
            }
        } else {
            self.walk(node, false);
        }
    }

    fn record_field_access(&mut self, node: &TsNode<'a>, is_write: bool) {
        let Some(prop) = node.field("property") else {
            return;
        };
        let Some(mut current) = node.field("object") else {
            return;
        };
        loop {
            match current.kind().as_ref() {
                "member_expression" => {
                    let Some(next) = current.field("object") else {
                        return;
                    };
                    current = next;
                }
                "parenthesized_expression"
                | "non_null_expression"
                | "as_expression"
                | "satisfies_expression" => {
                    let Some(next) = current.children().find(TsNode::is_named) else {
                        return;
                    };
                    current = next;
                }
                _ => break,
            }
        }
        if current.kind() == "identifier" {
            self.acc.push_field_access(FieldAccess {
                receiver_param: current.text().into_owned(),
                field_name: prop.text().into_owned(),
                is_write,
                span: self.tree.node_span(node),
            });
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
            self.acc.push_scoped_ident_ref(callee.clone());
            self.acc.push_call(CallSite {
                kind: CallKind::Scoped,
                callee,
                receiver: Some(CallReceiver::Named(ns_mod.to_string())),
                span,
            });
        } else if obj_name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_uppercase())
        {
            self.acc
                .push_constructed_type(TypeRef::Named(obj_name.clone()));
            self.acc.push_scoped_ident_ref(callee.clone());
            self.acc.push_call(CallSite {
                kind: CallKind::Scoped,
                callee,
                receiver: Some(CallReceiver::Named(obj_name)),
                span,
            });
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
            self.acc
                .push_constructed_type(TypeRef::Named(qualified.clone()));
            self.acc.push_scoped_ident_ref(callee.clone());
            self.acc.push_call(CallSite {
                kind: CallKind::Scoped,
                callee,
                receiver: Some(CallReceiver::Named(qualified)),
                span,
            });
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
