use super::TsNode;
use super::coupling;
use super::expr::{self, PatternReadTarget, TsBodyContext};
use super::jsx;
use super::normalize::TsNormalizeDialect;
use super::props::{self, ExtractedParams};
use super::syntax;
use crate::ast::item::{CallKind, CallReceiver, CallSite, FieldAccess, LocalBinding, TypeRef};
use crate::diagnostic::Span;
use crate::lang::common::{self, AccessMode, BodyAccumulator, BodyMetadataWalker};

pub(super) use crate::lang::common::ExtractedBody;

pub(super) struct TsBodyWalker<'a, 'b> {
    ctx: &'b TsBodyContext<'a, 'b>,
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
    pub(super) fn new(ctx: &'b TsBodyContext<'a, 'b>) -> Self {
        Self {
            ctx,
            acc: BodyAccumulator::default(),
        }
    }

    pub(super) fn extract_with_seeds(
        mut self,
        body: Option<TsNode<'a>>,
        local_bindings: Vec<LocalBinding>,
        default_exprs: &[TsNode<'a>],
    ) -> ExtractedBody {
        for binding in local_bindings {
            self.acc.push_local_binding(binding);
        }
        for default_expr in default_exprs {
            self.walk(default_expr, AccessMode::Read);
        }
        common::extract_body_with_dialect(self, TsNormalizeDialect, body)
    }

    pub(super) fn walk(&mut self, node: &TsNode<'a>, mode: AccessMode) {
        coupling::TsCouplingInspector::new(
            self.ctx.tree(),
            self.ctx.base_dir(),
            self.ctx.conventions(),
            self.acc.coupling_mut(),
        )
        .inspect(node);
        if let jsx::JsxWalkResult::Handled(children) =
            jsx::TsJsxWalker::new(self.ctx.tree(), self.ctx.namespace_imports(), &mut self.acc)
                .walk_jsx_node(node)
        {
            for child in children {
                self.walk(&child, AccessMode::Read);
            }
            return;
        }
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
                    && let Some(field_name) = syntax::extract_static_string_key(&index_node)
                    && let Some(raw_obj) = node.field("object")
                {
                    self.record_member_or_subscript_access(node, &raw_obj, field_name, mode);
                    return;
                }
            }
            "new_expression" => self.record_new_expression(node),
            "variable_declarator" => self.record_local_binding(node),
            "required_parameter" | "optional_parameter" => {
                self.record_nested_param_object_pattern(node);
            }
            _ => {}
        }

        if node.is_leaf() {
            if node.kind().as_ref() == "identifier" {
                self.acc.push_referenced_ident(node.text().into_owned());
            }
            return;
        }

        let next_mode = if expr::is_ts_transparent_node(node) {
            mode
        } else {
            AccessMode::Read
        };
        for child in node.children() {
            self.walk(&child, next_mode);
        }
    }

    fn record_nested_param_object_pattern(&mut self, node: &TsNode<'a>) {
        let Some(pat) = node.field("pattern") else {
            return;
        };
        if pat.kind() != "object_pattern" {
            return;
        }
        let type_node = node.field("type");
        let ty = type_node.as_ref().and_then(syntax::extract_type_ref);
        let read_target = PatternReadTarget {
            param: "",
            is_self: false,
            ty: ty.as_ref(),
        };
        self.ctx
            .collect_object_pattern_reads(&pat, read_target, &mut self.acc);
        let shape = props::resolve_param_prop_shape(
            type_node.as_ref(),
            None,
            self.ctx.prop_shapes(),
            self.ctx.conventions(),
        );
        let mut extracted = ExtractedParams::default();
        shape.extract_bindings(&pat, ty.as_ref(), self.ctx.prop_shapes(), &mut extracted);
        for binding in extracted.local_bindings {
            self.acc.push_local_binding(binding);
        }
    }

    fn record_member_or_subscript_access(
        &mut self,
        node: &TsNode<'a>,
        raw_obj: &TsNode<'a>,
        field_name: String,
        mode: AccessMode,
    ) {
        let target = expr::unwrap_ts_receiver(raw_obj);
        let (receiver_param, receiver_is_self, receiver_type, direct) = match target.kind().as_ref()
        {
            "this" => (String::new(), true, None, true),
            "identifier" => (target.text().into_owned(), false, None, true),
            _ => (
                String::new(),
                false,
                self.ctx.infer_expr_origin(&target).0,
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
            span: self.ctx.tree().node_span(node),
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
        if let Some(ty_ref) = expr::extract_ctor_type_ref(&ctor) {
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
        if name_node.kind() == "array_pattern" {
            if let Some(val) = node.field("value")
                && let Some(binding) = self.ctx.extract_state_hook_binding(&name_node, &val)
            {
                self.acc.push_local_binding(binding);
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
            self.acc.push_local_binding(LocalBinding {
                name,
                type_name: annotated,
                init_receiver: None,
                init_callee: None,
            });
            return;
        }
        if let Some(val) = node.field("value") {
            let (type_name, init_receiver, init_callee) = self.ctx.infer_expr_origin(&val);
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

    fn record_object_destructuring(&mut self, pattern: &TsNode<'a>, val: &TsNode<'a>) {
        let unwrapped = expr::unwrap_ts_receiver(val);
        let (receiver_param, receiver_is_self, receiver_type) = match unwrapped.kind().as_ref() {
            "this" => (String::new(), true, None),
            "identifier" => (unwrapped.text().into_owned(), false, None),
            _ => (
                String::new(),
                false,
                self.ctx.infer_expr_origin(&unwrapped).0,
            ),
        };
        let target = PatternReadTarget {
            param: &receiver_param,
            is_self: receiver_is_self,
            ty: receiver_type.as_ref(),
        };
        self.ctx
            .collect_object_pattern_reads(pattern, target, &mut self.acc);
    }

    fn record_call(&mut self, call_node: &TsNode<'a>) {
        let Some(func_node) = call_node.field("function") else {
            return;
        };
        let span = self.ctx.tree().node_span(call_node);
        if func_node.kind() == "import" {
            if let Some(target_mod) = self.ctx.extract_dynamic_import_target(call_node) {
                self.acc.push_call(CallSite {
                    kind: CallKind::Scoped,
                    callee: String::from("default"),
                    receiver: Some(CallReceiver::Named(target_mod.to_string())),
                    arg_receivers: Vec::new(),
                    span,
                });
            }
            return;
        }
        let arg_receivers = self.ctx.extract_arg_receivers(call_node);

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
                let Some(callee) = syntax::extract_static_string_key(&index_node) else {
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

    fn record_member_call(
        &mut self,
        raw_obj: &TsNode<'a>,
        callee: String,
        arg_receivers: Vec<Option<CallReceiver>>,
        span: Span,
    ) {
        let obj = expr::unwrap_ts_receiver(raw_obj);

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
                let receiver = if let (Some(ty), _, _) = self.ctx.infer_expr_origin(&obj) {
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
                    arg_receivers,
                    span,
                });
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
        if let Some(ns_mod) = self.ctx.namespace_imports().get(&obj_name) {
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
}
