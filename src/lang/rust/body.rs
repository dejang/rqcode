use super::TsNode;
use super::macro_tokens::{MacroTokenData, MacroTokenScanner};
use super::normalize::RustNormalizeDialect;
use super::syntax;
use crate::ast::SyntaxTree;
use crate::ast::item::{
    CallKind, CallReceiver, CallSite, FieldAccess, FieldInit, LocalBinding, TypeRef,
};
use crate::lang::common::{self, AccessMode, BodyAccumulator, BodyMetadataWalker, MemberFieldSpec};

pub(super) use crate::lang::common::ExtractedBody;

pub(super) struct BodyWalker<'a> {
    tree: &'a SyntaxTree,
    acc: BodyAccumulator,
}

impl<'a> BodyMetadataWalker<'a> for BodyWalker<'a> {
    fn walk_body(&mut self, body: &TsNode<'a>) {
        self.walk(body, AccessMode::Read);
    }

    fn into_accumulator(self) -> BodyAccumulator {
        self.acc
    }
}

impl<'a> BodyWalker<'a> {
    pub(super) fn new(tree: &'a SyntaxTree) -> Self {
        Self {
            tree,
            acc: BodyAccumulator::default(),
        }
    }

    pub(super) fn extract(self, body: Option<TsNode<'a>>) -> ExtractedBody {
        common::extract_body_with_dialect(self, RustNormalizeDialect, body)
    }

    fn walk(&mut self, node: &TsNode<'a>, mode: AccessMode) {
        match node.kind().as_ref() {
            "line_comment" | "block_comment" => return,
            "assignment_expression" | "compound_assignment_expr" => {
                let lhs_mode = if node.kind() == "compound_assignment_expr" {
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
            "reference_expression" => {
                let is_mut_ref = node.children().any(|c| c.kind() == "mutable_specifier");
                if let Some(val) = node.field("value") {
                    let next = if is_mut_ref {
                        AccessMode::ReadWrite
                    } else {
                        mode
                    };
                    self.walk(&val, next);
                }
                return;
            }
            "field_expression" => {
                let spec = MemberFieldSpec {
                    target_field: "value",
                    member_field: "field",
                    ident_kind: "identifier",
                    self_kind: Some("self"),
                    is_transparent: |kind| kind == "parenthesized_expression",
                };
                let fallback_ty = node.field("value").and_then(|v| infer_expr_origin(&v).0);
                if let Some((raw_value, recorded)) =
                    self.acc
                        .record_ident_field_access(self.tree, node, spec, mode, fallback_ty)
                {
                    let next = if mode.is_write() && !recorded {
                        AccessMode::ReadWrite
                    } else {
                        AccessMode::Read
                    };
                    self.walk(&raw_value, next);
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
            "scoped_identifier" => self.record_scoped_ident(node),
            "struct_expression" => self.record_struct_init(node),
            "struct_pattern" => self.record_struct_pattern(node),
            "tuple_struct_pattern" => self.record_tuple_struct_pattern(node),
            "let_declaration" => self.record_local_binding(node),
            "macro_invocation" => self.record_macro_invocation(node),
            _ => {}
        }

        if node.is_leaf() {
            if node.kind().as_ref() == "identifier" {
                self.acc.push_referenced_ident(node.text().into_owned());
            }
            return;
        }

        let next_mode = if node.kind() == "parenthesized_expression" {
            mode
        } else {
            AccessMode::Read
        };
        for child in node.children() {
            self.walk(&child, next_mode);
        }
    }

    fn record_scoped_ident(&mut self, node: &TsNode<'a>) {
        if let Some(name) = node.field("name") {
            self.acc.push_scoped_ident_ref(name.text().into_owned());
        }
        if let Some((prefix, sym)) = syntax::split_scoped_path(node)
            && prefix.len() >= 2
            && prefix[0] == "std"
            && match prefix[1].as_str() {
                "fs" | "process" | "env" => true,
                "net" => !matches!(
                    sym.as_str(),
                    "IpAddr" | "Ipv4Addr" | "Ipv6Addr" | "SocketAddr" | "SocketAddrV4" | "SocketAddrV6"
                ),
                _ => false,
            }
        {
            self.acc.coupling_mut().mark_ambient_io();
        }
    }

    fn record_macro_invocation(&mut self, node: &TsNode<'a>) {
        if let Some(hook) = super::macro_tokens::detect_cfg_test_hook(self.tree, node) {
            self.acc.coupling_mut().test_hooks.push(hook);
        }
        self.acc.coupling_mut().repr_assertions.extend(
            super::macro_tokens::detect_debug_repr_assertions(self.tree, node),
        );
        if let Some(tt) = node.children().find(|c| c.kind() == "token_tree") {
            let MacroTokenData {
                field_accesses,
                calls,
                constructed_types,
                field_inits,
                scoped_ident_refs,
            } = MacroTokenScanner::new(self.tree).scan_token_tree(&tt);
            self.acc.extend_macro_data(
                field_accesses,
                calls,
                constructed_types,
                field_inits,
                scoped_ident_refs,
            );
        }
    }

    /// Walk the callee expression of a `call_expression` without recording the method name itself
    /// as a struct field access.
    fn walk_call_callee(&mut self, node: &TsNode<'a>) {
        match node.kind().as_ref() {
            "field_expression" => {
                for child in node.children() {
                    self.walk(&child, AccessMode::Read);
                }
            }
            "generic_function" => {
                let func_range = node.field("function").map(|f| f.range());
                for child in node.children() {
                    if func_range == Some(child.range()) {
                        self.walk_call_callee(&child);
                    } else {
                        self.walk(&child, AccessMode::Read);
                    }
                }
            }
            _ => self.walk(node, AccessMode::Read),
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

    fn record_struct_pattern(&mut self, node: &TsNode<'a>) {
        let Some(ty_node) = node.field("type") else {
            return;
        };
        let raw_ty = ty_node.text();
        let ty_name = strip_turbofish(&raw_ty);
        if ty_name.is_empty() {
            return;
        }
        let ty_ref = syntax::rust_type_ref(ty_name);
        for child in node.children() {
            if child.kind() != "field_pattern"
                || child
                    .field("pattern")
                    .is_some_and(|pat| pat.text().trim() == "_")
            {
                continue;
            }
            let field_name = child
                .field("name")
                .map(|n| n.text().into_owned())
                .or_else(|| {
                    child
                        .children()
                        .find(|c| c.kind() == "shorthand_field_identifier")
                        .map(|n| n.text().into_owned())
                });
            if let Some(field_name) = field_name {
                self.acc.push_field_access(FieldAccess {
                    receiver_param: String::new(),
                    receiver_is_self: false,
                    receiver_type: Some(ty_ref.clone()),
                    field_name,
                    is_read: true,
                    is_write: false,
                    span: self.tree.node_span(&child),
                });
            }
        }
    }

    fn record_tuple_struct_pattern(&mut self, node: &TsNode<'a>) {
        let Some(ty_node) = node.field("type") else {
            return;
        };
        let ty_range = ty_node.range();
        let raw_ty = ty_node.text();
        let ty_name = strip_turbofish(&raw_ty);
        if ty_name.is_empty() {
            return;
        }
        let ty_ref = syntax::rust_type_ref(ty_name);
        let mut idx = 0usize;
        for child in node.children() {
            if child.kind() == "remaining_field_pattern" {
                break;
            }
            if !child.is_named()
                || child.range() == ty_range
                || matches!(child.kind().as_ref(), "line_comment" | "block_comment")
            {
                continue;
            }
            if child.text().trim() != "_" {
                self.acc.push_field_access(FieldAccess {
                    receiver_param: String::new(),
                    receiver_is_self: false,
                    receiver_type: Some(ty_ref.clone()),
                    field_name: idx.to_string(),
                    is_read: true,
                    is_write: false,
                    span: self.tree.node_span(&child),
                });
            }
            idx += 1;
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
                init_receiver: None,
                init_callee: None,
            });
            return;
        }

        if let Some(val) = node.field("value") {
            let (type_name, init_receiver, init_callee) = infer_expr_origin(&val);
            if type_name.is_some() || init_callee.is_some() {
                self.acc.push_local_binding(LocalBinding {
                    name,
                    type_name,
                    init_receiver,
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
        let arg_receivers: Vec<Option<CallReceiver>> = call_node
            .field("arguments")
            .map(|args| {
                args.children()
                    .filter(TsNode::is_named)
                    .map(|arg| extract_expr_receiver(&arg))
                    .collect()
            })
            .unwrap_or_default();

        match func_node.kind().as_ref() {
            "identifier" => {
                self.acc.push_call(CallSite {
                    kind: CallKind::Free,
                    callee: func_node.text().into_owned(),
                    receiver: None,
                    arg_receivers,
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
                    } else if let (Some(ty), _, _) = infer_expr_origin(&v) {
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
                        arg_receivers,
                        span,
                    });
                }
            }
            "scoped_identifier" => {
                if let Some((prefix, callee)) = syntax::split_scoped_path(&func_node) {
                    if prefix.len() == 1 && matches!(prefix[0].as_str(), "super" | "self") {
                        self.acc.push_call(CallSite {
                            kind: CallKind::SameModule,
                            callee,
                            receiver: None,
                            arg_receivers,
                            span,
                        });
                        return;
                    }
                    let normalized: Vec<&str> = prefix
                        .iter()
                        .map(String::as_str)
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
                        callee,
                        receiver: syntax::rust_scoped_receiver(ty_path),
                        arg_receivers,
                        span,
                    });
                }
            }
            _ => {}
        }
    }
}

fn strip_turbofish(raw: &str) -> &str {
    raw.split("::<").next().unwrap_or(raw).trim()
}

fn extract_expr_receiver(node: &TsNode<'_>) -> Option<CallReceiver> {
    match node.kind().as_ref() {
        "reference_expression" => node.field("value").and_then(|v| extract_expr_receiver(&v)),
        "parenthesized_expression" | "try_expression" | "await_expression" => node
            .children()
            .find(TsNode::is_named)
            .and_then(|c| extract_expr_receiver(&c)),
        "self" => Some(CallReceiver::SelfInstance),
        "identifier" => Some(CallReceiver::Named(node.text().into_owned())),
        _ => match infer_expr_origin(node).0 {
            Some(TypeRef::SelfType) => Some(CallReceiver::SelfType),
            Some(TypeRef::Named(name)) => Some(CallReceiver::Named(name)),
            None => None,
        },
    }
}

fn infer_expr_origin(node: &TsNode<'_>) -> (Option<TypeRef>, Option<CallReceiver>, Option<String>) {
    match node.kind().as_ref() {
        "reference_expression" => node
            .field("value")
            .map_or((None, None, None), |v| infer_expr_origin(&v)),
        "try_expression" | "parenthesized_expression" | "await_expression" => node
            .children()
            .find(TsNode::is_named)
            .map_or((None, None, None), |c| infer_expr_origin(&c)),
        "identifier" => {
            let text = node.text();
            if text.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                (Some(syntax::rust_type_ref(text.into_owned())), None, None)
            } else {
                (None, None, None)
            }
        }
        "scoped_identifier" => {
            if let Some((prefix, name)) = syntax::split_scoped_path(node)
                && name.chars().next().is_some_and(|c| c.is_ascii_uppercase())
            {
                let mut full = prefix;
                full.push(name);
                return (Some(syntax::rust_type_ref(full.join("::"))), None, None);
            }
            (None, None, None)
        }
        "struct_expression" => {
            let ty = node
                .field("name")
                .map(|n| syntax::rust_type_ref(strip_turbofish(&n.text())));
            (ty, None, None)
        }
        "call_expression" => {
            let Some(raw_func) = node.field("function") else {
                return (None, None, None);
            };
            let func_node = if raw_func.kind() == "generic_function" {
                let Some(inner) = raw_func.field("function") else {
                    return (None, None, None);
                };
                inner
            } else {
                raw_func
            };
            match func_node.kind().as_ref() {
                "identifier" => (None, None, Some(func_node.text().into_owned())),
                "field_expression" => {
                    let method = func_node
                        .field("field")
                        .map(|f| f.text().into_owned())
                        .unwrap_or_default();
                    if let Some(recv) = func_node.field("value") {
                        if syntax::is_transparent_chain_method(method.as_str()) {
                            return infer_expr_origin(&recv);
                        }
                        if !method.is_empty()
                            && let Some(call_recv) = extract_expr_receiver(&recv)
                        {
                            return (None, Some(call_recv), Some(method));
                        }
                    }
                    (None, None, None)
                }
                "scoped_identifier" => {
                    let Some((prefix, callee)) = syntax::split_scoped_path(&func_node) else {
                        return (None, None, None);
                    };
                    if let Some(owner) = prefix.last()
                        && owner.chars().next().is_some_and(|c| c.is_ascii_uppercase())
                    {
                        if matches!(
                            owner.as_str(),
                            "Box" | "Arc" | "Rc" | "Mutex" | "RwLock" | "RefCell"
                        ) && callee == "new"
                            && let Some(args) = node.field("arguments")
                            && let Some(first_arg) = args.children().find(TsNode::is_named)
                        {
                            return infer_expr_origin(&first_arg);
                        }
                        return (Some(syntax::rust_type_ref(prefix.join("::"))), None, None);
                    }
                    (None, None, Some(callee))
                }
                _ => (None, None, None),
            }
        }
        _ => (None, None, None),
    }
}
