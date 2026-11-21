use super::TsNode;
use super::macro_tokens::{MacroTokenData, MacroTokenScanner};
use super::syntax;
use crate::ast::SyntaxTree;
use crate::ast::item::{
    CallKind, CallReceiver, CallSite, FieldAccess, FieldInit, LocalBinding, TypeRef,
};
use crate::ast::vector::NormToken;
use crate::lang::common::{BodyAccumulator, LeafClass};

pub(super) use crate::lang::common::ExtractedBody;

pub(super) struct BodyWalker<'a> {
    tree: &'a SyntaxTree,
    acc: BodyAccumulator,
}

impl<'a> BodyWalker<'a> {
    pub(super) fn new(tree: &'a SyntaxTree) -> Self {
        Self {
            tree,
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
        let kind = node.kind();
        match kind.as_ref() {
            "line_comment" | "block_comment" => return,
            "assignment_expression" | "compound_assignment_expr" => {
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
                return;
            }
            "reference_expression" => {
                let is_mut_ref = node.children().any(|c| c.kind() == "mutable_specifier");
                self.acc.push_token(if is_mut_ref {
                    NormToken::RefMut
                } else {
                    NormToken::Ref
                });
                if let Some(val) = node.field("value") {
                    self.walk(&val, in_write_target || is_mut_ref);
                }
                return;
            }
            "field_expression" => {
                self.record_field_access(node, in_write_target);
                self.acc.push_token(NormToken::Field);
            }
            "call_expression" => {
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
                return;
            }
            "scoped_identifier" => {
                if let Some(name) = node.field("name") {
                    self.acc.push_scoped_ident_ref(name.text().into_owned());
                }
            }
            "struct_expression" => {
                self.record_struct_init(node);
                self.acc.push_token(NormToken::Construct);
            }
            "let_declaration" => {
                self.record_local_binding(node);
                self.acc.push_token(NormToken::Bind);
            }
            "macro_invocation" => {
                self.record_macro_invocation(node);
                self.acc.push_token(NormToken::Macro);
            }
            "if_expression" => self.acc.push_token(NormToken::If),
            "match_expression" => self.acc.push_token(NormToken::Branch),
            "match_arm" => self.acc.push_token(NormToken::BranchArm),
            "for_expression" | "while_expression" | "loop_expression" => {
                self.acc.push_token(NormToken::Loop);
            }
            "return_expression" => self.acc.push_token(NormToken::Return),
            "closure_expression" => self.acc.push_token(NormToken::Closure),
            "try_expression" => self.acc.push_token(NormToken::Try),
            _ => {}
        }

        if node.is_leaf() {
            let class =
                match kind.as_ref() {
                    "identifier"
                    | "field_identifier"
                    | "type_identifier"
                    | "shorthand_field_identifier"
                    | "lifetime" => LeafClass::Ident,
                    "integer_literal" | "float_literal" | "string_literal" | "string_content"
                    | "raw_string_literal" | "char_literal" | "boolean_literal" | "true"
                    | "false" => LeafClass::Lit,
                    "(" | ")" | "{" | "}" | "[" | "]" | ";" | "," | "." | ":" | "::" | "\""
                    | "'" => LeafClass::Skip,
                    _ => LeafClass::Other,
                };
            self.acc.record_leaf(node, class);
            return;
        }

        for child in node.children() {
            self.walk(&child, false);
        }
    }

    fn record_field_access(&mut self, node: &TsNode<'a>, is_write: bool) {
        let Some(field) = node.field("field") else {
            return;
        };
        let Some(mut current) = node.field("value") else {
            return;
        };
        while current.kind() == "field_expression" {
            let Some(next) = current.field("value") else {
                return;
            };
            current = next;
        }
        if current.kind() == "identifier" {
            self.acc.push_field_access(FieldAccess {
                receiver_param: current.text().into_owned(),
                field_name: field.text().into_owned(),
                is_write,
                span: self.tree.node_span(node),
            });
        }
    }

    fn record_macro_invocation(&mut self, node: &TsNode<'a>) {
        self.acc.collect_scoped_idents(node);
        if let Some(tt) = node.children().find(|c| c.kind() == "token_tree") {
            let MacroTokenData {
                field_accesses,
                calls,
                constructed_types,
                field_inits,
            } = MacroTokenScanner::new(self.tree).scan_token_tree(&tt);
            self.acc
                .extend_macro_data(field_accesses, calls, constructed_types, field_inits);
        }
    }

    /// Walk the callee expression of a `call_expression` without recording the method name itself
    /// as a struct field access, while preserving the normalized token stream.
    fn walk_call_callee(&mut self, node: &TsNode<'a>) {
        match node.kind().as_ref() {
            "field_expression" => {
                self.acc.push_token(NormToken::Field);
                for child in node.children() {
                    self.walk(&child, false);
                }
            }
            "generic_function" => {
                let func_range = node.field("function").map(|f| f.range());
                for child in node.children() {
                    if func_range == Some(child.range()) {
                        self.walk_call_callee(&child);
                    } else {
                        self.walk(&child, false);
                    }
                }
            }
            _ => self.walk(node, false),
        }
    }

    fn record_struct_init(&mut self, node: &TsNode<'a>) {
        let Some(name_node) = node.field("name") else {
            return;
        };
        let raw_name = name_node.text();
        let ty_name = raw_name
            .split("::<")
            .next()
            .unwrap_or(raw_name.as_ref())
            .trim();
        if ty_name.is_empty() {
            return;
        }
        let ty_ref = syntax::rust_type_ref(ty_name);
        self.acc.push_constructed_type(ty_ref.clone());

        let Some(body) = node.field("body") else {
            return;
        };
        for child in body.children() {
            match child.kind().as_ref() {
                "field_initializer" => {
                    if let Some(field) = child.field("field") {
                        self.acc.push_field_init(FieldInit {
                            type_name: ty_ref.clone(),
                            field_name: field.text().into_owned(),
                            span: self.tree.node_span(&child),
                        });
                    }
                }
                "shorthand_field_initializer" => {
                    self.acc.push_field_init(FieldInit {
                        type_name: ty_ref.clone(),
                        field_name: child.text().into_owned(),
                        span: self.tree.node_span(&child),
                    });
                }
                _ => {}
            }
        }
    }

    fn record_local_binding(&mut self, node: &TsNode<'a>) {
        let Some(pattern) = node.field("pattern") else {
            return;
        };
        let Some(name) = syntax::extract_pattern_ident(&pattern) else {
            return;
        };

        let annotated_type = node
            .field("type")
            .and_then(|t| syntax::extract_base_type(&t).0);
        if annotated_type.is_some() {
            self.acc.push_local_binding(LocalBinding {
                name,
                type_name: annotated_type,
                init_callee: None,
            });
            return;
        }

        if let Some(val) = node.field("value") {
            let (type_name, init_callee) = infer_expr_origin(&val);
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
        let Some(raw_func) = call_node.field("function") else {
            return;
        };
        let func_node = if raw_func.kind() == "generic_function" {
            let Some(inner) = raw_func.field("function") else {
                return;
            };
            inner
        } else {
            raw_func
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
            "field_expression" => {
                let callee = func_node
                    .field("field")
                    .map(|f| f.text().into_owned())
                    .unwrap_or_default();
                let receiver = func_node.field("value").map(|v| {
                    if v.kind() == "self" {
                        CallReceiver::SelfInstance
                    } else if v.kind() == "identifier" {
                        CallReceiver::Named(v.text().into_owned())
                    } else if let (Some(ty), _) = infer_expr_origin(&v) {
                        match ty {
                            TypeRef::SelfType => CallReceiver::SelfType,
                            TypeRef::Named(name) => CallReceiver::Named(name),
                        }
                    } else {
                        CallReceiver::Named(v.text().into_owned())
                    }
                });
                if !callee.is_empty() {
                    self.acc.push_call(CallSite {
                        kind: CallKind::Method,
                        callee,
                        receiver,
                        span,
                    });
                }
            }
            "scoped_identifier" => {
                let text = func_node.text();
                let parts: Vec<&str> = text.split("::").filter(|s| !s.is_empty()).collect();
                if let Some((&callee, prefix)) = parts.split_last() {
                    if matches!(prefix, ["super" | "self"]) {
                        self.acc.push_call(CallSite {
                            kind: CallKind::SameModule,
                            callee: callee.to_string(),
                            receiver: None,
                            span,
                        });
                        return;
                    }
                    let normalized: Vec<&str> = prefix
                        .iter()
                        .copied()
                        .filter(|&s| s != "self" && s != "super")
                        .collect();
                    let ty_path = normalized.join("::");
                    if let Some(&maybe_type) = normalized.last()
                        && maybe_type
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_uppercase())
                    {
                        self.acc
                            .push_constructed_type(syntax::rust_type_ref(ty_path.as_str()));
                    }
                    self.acc.push_call(CallSite {
                        kind: CallKind::Scoped,
                        callee: callee.to_string(),
                        receiver: syntax::rust_scoped_receiver(ty_path),
                        span,
                    });
                }
            }
            _ => {}
        }
    }
}

fn infer_expr_origin(node: &TsNode<'_>) -> (Option<TypeRef>, Option<String>) {
    match node.kind().as_ref() {
        "reference_expression" => node
            .field("value")
            .map_or((None, None), |v| infer_expr_origin(&v)),
        "try_expression" | "parenthesized_expression" | "await_expression" => node
            .children()
            .find(TsNode::is_named)
            .map_or((None, None), |c| infer_expr_origin(&c)),
        "struct_expression" => {
            let ty = node.field("name").map(|n| {
                let raw = n.text();
                let name = raw.split("::<").next().unwrap_or(raw.as_ref()).trim();
                syntax::rust_type_ref(name)
            });
            (ty, None)
        }
        "call_expression" => {
            let Some(raw_func) = node.field("function") else {
                return (None, None);
            };
            let func_node = if raw_func.kind() == "generic_function" {
                let Some(inner) = raw_func.field("function") else {
                    return (None, None);
                };
                inner
            } else {
                raw_func
            };

            match func_node.kind().as_ref() {
                "identifier" => (None, Some(func_node.text().into_owned())),
                "field_expression" => {
                    let method = func_node
                        .field("field")
                        .map(|f| f.text().into_owned())
                        .unwrap_or_default();
                    if matches!(
                        method.as_str(),
                        "unwrap"
                            | "expect"
                            | "clone"
                            | "unwrap_or_default"
                            | "as_ref"
                            | "as_mut"
                            | "borrow"
                            | "borrow_mut"
                    ) && let Some(recv) = func_node.field("value")
                    {
                        return infer_expr_origin(&recv);
                    }
                    (None, None)
                }
                "scoped_identifier" => {
                    let text = func_node.text();
                    let parts: Vec<&str> = text.split("::").filter(|s| !s.is_empty()).collect();
                    let Some((&callee, prefix)) = parts.split_last() else {
                        return (None, None);
                    };
                    if let Some(&owner) = prefix.last()
                        && owner.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                    {
                        if matches!(owner, "Box" | "Arc" | "Rc" | "Mutex" | "RwLock" | "RefCell")
                            && callee == "new"
                            && let Some(args) = node.field("arguments")
                            && let Some(first_arg) = args.children().find(TsNode::is_named)
                        {
                            return infer_expr_origin(&first_arg);
                        }
                        return (Some(syntax::rust_type_ref(prefix.join("::"))), None);
                    }
                    (None, Some(callee.to_string()))
                }
                _ => (None, None),
            }
        }
        _ => (None, None),
    }
}
