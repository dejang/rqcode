use super::TsNode;
use super::conventions::TypeScriptConventions;
use super::props::PropShapeMap;
use super::syntax;
use crate::ast::item::{CallReceiver, FieldAccess, LocalBinding, TypeRef};
use crate::ast::{ModulePath, SyntaxTree};
use crate::lang::common::{self, BodyAccumulator, is_comment_kind};
use std::collections::HashMap;

/// Target receiver metadata for destructured object-pattern field reads.
#[derive(Copy, Clone)]
pub(super) struct PatternReadTarget<'a> {
    pub(super) param: &'a str,
    pub(super) is_self: bool,
    pub(super) ty: Option<&'a TypeRef>,
}

/// Shared immutable context threaded through TypeScript/TSX body, member, and JSX walkers.
pub(super) struct TsBodyContext<'a, 'b> {
    tree: &'a SyntaxTree,
    base_dir: &'b [String],
    namespace_imports: &'b HashMap<String, ModulePath>,
    conventions: &'a TypeScriptConventions,
    prop_shapes: &'b PropShapeMap,
}

impl<'a, 'b> TsBodyContext<'a, 'b> {
    pub(super) fn new(
        tree: &'a SyntaxTree,
        base_dir: &'b [String],
        namespace_imports: &'b HashMap<String, ModulePath>,
        conventions: &'a TypeScriptConventions,
        prop_shapes: &'b PropShapeMap,
    ) -> Self {
        Self {
            tree,
            base_dir,
            namespace_imports,
            conventions,
            prop_shapes,
        }
    }

    pub(super) fn tree(&self) -> &'a SyntaxTree {
        self.tree
    }

    pub(super) fn base_dir(&self) -> &'b [String] {
        self.base_dir
    }

    pub(super) fn namespace_imports(&self) -> &'b HashMap<String, ModulePath> {
        self.namespace_imports
    }

    pub(super) fn conventions(&self) -> &'a TypeScriptConventions {
        self.conventions
    }

    pub(super) fn prop_shapes(&self) -> &'b PropShapeMap {
        self.prop_shapes
    }

    pub(super) fn infer_expr_origin(
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

    pub(super) fn extract_arg_receivers(
        &self,
        call_node: &TsNode<'_>,
    ) -> Vec<Option<CallReceiver>> {
        let Some(args) = call_node.field("arguments") else {
            return Vec::new();
        };
        args.children()
            .filter(TsNode::is_named)
            .map(|arg| {
                let unwrapped = unwrap_ts_receiver(&arg);
                match unwrapped.kind().as_ref() {
                    "this" => Some(CallReceiver::SelfInstance),
                    "identifier" => Some(CallReceiver::Named(unwrapped.text().into_owned())),
                    _ => match self.infer_expr_origin(&unwrapped).0 {
                        Some(TypeRef::SelfType) => Some(CallReceiver::SelfType),
                        Some(TypeRef::Named(name)) => Some(CallReceiver::Named(name)),
                        None => None,
                    },
                }
            })
            .collect()
    }

    pub(super) fn extract_state_hook_binding(
        &self,
        pattern: &TsNode<'_>,
        val: &TsNode<'_>,
    ) -> Option<LocalBinding> {
        if val.kind() != "call_expression" {
            return None;
        }
        let func_node = val.field("function")?;
        let callee_name = match func_node.kind().as_ref() {
            "identifier" => func_node.text(),
            "member_expression" => func_node.field("property")?.text(),
            _ => return None,
        };
        if !self.conventions.is_state_hook(callee_name.as_ref()) {
            return None;
        }
        let first_elem = pattern
            .children()
            .find(|c| c.is_named() && !is_comment_kind(c.kind().as_ref()))?;
        if first_elem.kind() != "identifier" {
            return None;
        }
        let type_name = val
            .field("type_arguments")
            .and_then(|args| {
                args.children()
                    .find(|c| c.is_named() && !is_comment_kind(c.kind().as_ref()))
                    .and_then(|arg| syntax::extract_type_ref(&arg))
            })
            .or_else(|| {
                val.field("arguments")
                    .and_then(|args| {
                        args.children()
                            .find(|c| c.is_named() && !is_comment_kind(c.kind().as_ref()))
                    })
                    .and_then(|first_arg| self.infer_expr_origin(&first_arg).0)
            })?;
        Some(LocalBinding {
            name: first_elem.text().into_owned(),
            type_name: Some(type_name),
            init_receiver: None,
            init_callee: None,
        })
    }

    pub(super) fn collect_object_pattern_reads(
        &self,
        pattern: &TsNode<'_>,
        target: PatternReadTarget<'_>,
        acc: &mut BodyAccumulator,
    ) {
        let push_read = |field_name: String, node: &TsNode<'_>, acc: &mut BodyAccumulator| {
            if !target.is_self && target.param.is_empty() && target.ty.is_none() {
                return;
            }
            acc.push_field_access(FieldAccess {
                receiver_param: target.param.to_string(),
                receiver_is_self: target.is_self,
                receiver_type: target.ty.cloned(),
                field_name,
                is_read: true,
                is_write: false,
                span: self.tree.node_span(node),
            });
        };

        for child in pattern.children() {
            match child.kind().as_ref() {
                "shorthand_property_identifier_pattern" => {
                    push_read(child.text().into_owned(), &child, acc);
                }
                "object_assignment_pattern" => {
                    if let Some(left) = child.field("left") {
                        match left.kind().as_ref() {
                            "shorthand_property_identifier_pattern" | "identifier" => {
                                push_read(left.text().into_owned(), &child, acc);
                            }
                            "object_pattern" => {
                                self.collect_object_pattern_reads(&left, target, acc);
                            }
                            _ => {}
                        }
                    }
                }
                "pair_pattern" => {
                    let field_name = child.field("key").map(|key| {
                        syntax::extract_static_string_key(&key)
                            .unwrap_or_else(|| key.text().into_owned())
                    });
                    if let Some(ref name) = field_name {
                        push_read(name.clone(), &child, acc);
                    }
                    if let Some(mut val) = child.field("value") {
                        if val.kind() == "assignment_pattern"
                            && let Some(left) = val.field("left")
                        {
                            val = left;
                        }
                        if val.kind() == "object_pattern"
                            && let Some(nested_ty) = field_name.as_deref().and_then(|fname| {
                                let TypeRef::Named(owner_name) = target.ty? else {
                                    return None;
                                };
                                self.prop_shapes.get(owner_name)?.field_type(fname)
                            })
                        {
                            let nested = PatternReadTarget {
                                param: "",
                                is_self: false,
                                ty: Some(nested_ty),
                            };
                            self.collect_object_pattern_reads(&val, nested, acc);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    /// Resolve a dynamic `import('./relative')` call expression to a project [`ModulePath`].
    pub(super) fn extract_dynamic_import_target(
        &self,
        call_node: &TsNode<'_>,
    ) -> Option<ModulePath> {
        let func_node = call_node.field("function")?;
        if func_node.kind() != "import" {
            return None;
        }
        let args = call_node.field("arguments")?;
        let first_arg = args
            .children()
            .find(|c| c.is_named() && !is_comment_kind(c.kind().as_ref()))?;
        let spec = syntax::extract_static_string_key(&first_arg)?;
        if self.conventions.is_asset_specifier(&spec) {
            return None;
        }
        syntax::resolve_relative_module_path(self.base_dir, &spec)
    }
}

pub(super) fn is_ts_transparent_node(node: &TsNode<'_>) -> bool {
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

pub(super) fn unwrap_ts_receiver<'a>(node: &TsNode<'a>) -> TsNode<'a> {
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

pub(super) fn extract_ctor_type_ref(ctor: &TsNode<'_>) -> Option<TypeRef> {
    match ctor.kind().as_ref() {
        "identifier" => Some(TypeRef::Named(ctor.text().into_owned())),
        "member_expression" => syntax::join_dotted_path(&ctor.text()).map(TypeRef::Named),
        _ => None,
    }
}
