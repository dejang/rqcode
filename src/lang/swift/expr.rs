use super::{SwiftNode, syntax};
use crate::ast::item::TypeRef;

pub(super) enum SwiftCondition<'a> {
    /// `let pat = rhs` / `var pat = rhs`, or shorthand `let x` when `rhs` is empty.
    OptionalBinding {
        pattern: Vec<SwiftNode<'a>>,
        bound_names: Vec<String>,
        rhs: Vec<SwiftNode<'a>>,
    },
    /// `case pat = rhs`
    CaseBinding {
        pattern: Vec<SwiftNode<'a>>,
        bound_names: Vec<String>,
        rhs: Vec<SwiftNode<'a>>,
    },
    /// Plain boolean expression or availability condition.
    Expr(Vec<SwiftNode<'a>>),
}

pub(super) struct SwiftConditionalParts<'a> {
    conditions: Vec<SwiftCondition<'a>>,
    then_children: Vec<SwiftNode<'a>>,
    else_children: Vec<SwiftNode<'a>>,
}

impl<'a> SwiftConditionalParts<'a> {
    /// Split an `if_statement`, `guard_statement`, or `while_statement` into its ordered
    /// condition items, `then`/loop children, and `else` children so both the semantic body
    /// walker and normalizer apply identical lexical scoping.
    pub(super) fn parse(node: &SwiftNode<'a>) -> Self {
        let children: Vec<SwiftNode<'a>> = node.children().collect();
        let body_start = children
            .iter()
            .position(|c| matches!(c.kind().as_ref(), "{" | "else"))
            .unwrap_or(children.len());

        let mut conditions = Vec::new();
        let mut cursor = 1;
        while cursor < body_start {
            let seg_start = cursor;
            let mut eq_pos = None;
            let mut has_value_binding = false;
            let mut paren_depth = 0usize;
            while cursor < body_start {
                let child = &children[cursor];
                if !child.is_named() {
                    match child.text().trim() {
                        "(" | "[" => paren_depth += 1,
                        ")" | "]" => paren_depth = paren_depth.saturating_sub(1),
                        "," if paren_depth == 0 => break,
                        "=" if paren_depth == 0 && eq_pos.is_none() => {
                            eq_pos = Some(cursor - seg_start);
                        }
                        _ => {}
                    }
                }
                if child.kind() == "value_binding_pattern" {
                    has_value_binding = true;
                }
                cursor += 1;
            }
            let seg = &children[seg_start..cursor];
            if cursor < body_start {
                cursor += 1;
            }
            if seg.is_empty() {
                continue;
            }
            if seg
                .first()
                .is_some_and(|n| !n.is_named() && n.text().trim() == "case")
            {
                let (pat_slice, rhs_slice) = match eq_pos {
                    Some(eq) => (&seg[1..eq], &seg[eq + 1..]),
                    None => (&seg[1..], &[][..]),
                };
                let mut bound_names = Vec::new();
                collect_bound_names_iter(pat_slice.iter().cloned(), false, &mut bound_names);
                conditions.push(SwiftCondition::CaseBinding {
                    pattern: pat_slice.to_vec(),
                    bound_names,
                    rhs: rhs_slice.to_vec(),
                });
            } else if has_value_binding {
                let (lhs_raw, rhs_slice) = match eq_pos {
                    Some(eq) => (&seg[..eq], &seg[eq + 1..]),
                    None => (seg, &[][..]),
                };
                let mut bound_names = Vec::new();
                for item in lhs_raw {
                    if matches!(item.kind().as_ref(), "pattern" | "simple_identifier") {
                        collect_pattern_names(item, &mut bound_names);
                    }
                }
                conditions.push(SwiftCondition::OptionalBinding {
                    pattern: lhs_raw.to_vec(),
                    bound_names,
                    rhs: rhs_slice.to_vec(),
                });
            } else {
                conditions.push(SwiftCondition::Expr(seg.to_vec()));
            }
        }

        let tail = &children[body_start..];
        let (then_children, else_children) = match tail.iter().position(|c| c.kind() == "else") {
            Some(e) => (tail[..e].to_vec(), tail[e + 1..].to_vec()),
            None => (tail.to_vec(), Vec::new()),
        };
        Self {
            conditions,
            then_children,
            else_children,
        }
    }

    pub(super) fn conditions(&self) -> &[SwiftCondition<'a>] {
        &self.conditions
    }

    pub(super) fn then_children(&self) -> &[SwiftNode<'a>] {
        &self.then_children
    }

    pub(super) fn else_children(&self) -> &[SwiftNode<'a>] {
        &self.else_children
    }
}

pub(super) fn extract_pattern_name(pattern: &SwiftNode<'_>) -> Option<String> {
    if let Some(bound) = pattern.field("bound_identifier") {
        let text = bound.text();
        if !text.is_empty() {
            return Some(text.into_owned());
        }
    }
    if pattern.kind() == "simple_identifier" {
        return Some(pattern.text().into_owned());
    }
    for child in pattern.children() {
        if child.kind() == "simple_identifier" {
            return Some(child.text().into_owned());
        }
        if child.kind() == "pattern"
            && let Some(nested) = extract_pattern_name(&child)
        {
            return Some(nested);
        }
    }
    None
}

pub(super) fn collect_pattern_names(pattern: &SwiftNode<'_>, out: &mut Vec<String>) {
    if let Some(bound) = pattern.field("bound_identifier") {
        let text = bound.text();
        if !text.is_empty() && text.as_ref() != "_" {
            out.push(text.into_owned());
            return;
        }
    }
    if pattern.kind() == "simple_identifier" {
        let text = pattern.text();
        if !text.is_empty() && text.as_ref() != "_" {
            out.push(text.into_owned());
        }
        return;
    }
    let mut children = pattern.children().peekable();
    while let Some(node) = children.next() {
        if !node.is_named() && node.text().trim() == "." {
            let _ = children.next();
            continue;
        }
        if node.kind() == "simple_identifier"
            && children
                .peek()
                .is_some_and(|n| !n.is_named() && n.text().trim() == ":")
        {
            let _ = children.next();
            continue;
        }
        if matches!(node.kind().as_ref(), "pattern" | "simple_identifier") {
            collect_pattern_names(&node, out);
        }
    }
}

/// Extract identifiers explicitly bound in a `switch_entry`'s `case` patterns.
pub(super) fn extract_switch_entry_bindings(entry: &SwiftNode<'_>) -> Vec<String> {
    let mut vars = Vec::new();
    for child in entry.children().filter(|c| c.kind() == "switch_pattern") {
        collect_bound_names_node(&child, false, &mut vars);
    }
    vars
}

/// Extract identifiers bound by a `catch_block` (either explicit pattern bindings or Swift's
/// implicit `error` binding when no catch pattern is written).
pub(super) fn extract_catch_bindings(catch_block: &SwiftNode<'_>) -> Vec<String> {
    let mut vars = Vec::new();
    match catch_block.field("error") {
        Some(err_pat) => collect_bound_names_node(&err_pat, false, &mut vars),
        None => vars.push(String::from("error")),
    }
    vars
}

/// Extract parameter names declared on a nested local `function_declaration`.
pub(super) fn extract_nested_func_params(func_node: &SwiftNode<'_>) -> Vec<String> {
    func_node
        .children()
        .filter(|c| c.kind() == "parameter")
        .filter_map(|p| p.field("name").map(|n| n.text().into_owned()))
        .collect()
}

/// Extract closure capture-list outer read expressions and newly shadowed capture names.
pub(super) fn extract_closure_capture_bindings<'a>(
    lambda: &SwiftNode<'a>,
) -> (Vec<SwiftNode<'a>>, Vec<String>) {
    let mut reads = Vec::new();
    let mut bound = Vec::new();
    for cap_list in lambda.children().filter(|c| c.kind() == "capture_list") {
        for item in cap_list
            .children()
            .filter(|c| c.kind() == "capture_list_item")
        {
            let name_node = item.field("name");
            if let Some(val) = item.field("value") {
                reads.push(val);
                if let Some(name) = name_node {
                    let text = name.text();
                    if !text.is_empty() && text.as_ref() != "self" {
                        bound.push(text.into_owned());
                    }
                }
            } else if let Some(name) = name_node {
                reads.push(name.clone());
                if name.kind() == "simple_identifier" {
                    let text = name.text();
                    if !text.is_empty() && text.as_ref() != "self" {
                        bound.push(text.into_owned());
                    }
                }
            }
        }
    }
    (reads, bound)
}

/// True when a `call_expression` is a Swift subscript expression (`arr[i]`), which
/// `tree-sitter-swift` parses as `call_expression(callee, call_suffix(value_arguments("[", ...)))`.
pub(super) fn is_subscript_call(call_node: &SwiftNode<'_>) -> bool {
    call_node
        .children()
        .find(|c| c.kind() == "call_suffix")
        .and_then(|suffix| suffix.children().find(|c| c.kind() == "value_arguments"))
        .and_then(|args| args.children().next())
        .is_some_and(|tok| !tok.is_named() && tok.text().trim() == "[")
}

/// True when `node` is a key-path expression (`\.customer.name`, `\Account.bonus`) or a
/// chained navigation rooted at one whose suffix is not a key-path component (e.g., `\T.pair.0`).
pub(super) fn is_key_path_chain(node: &SwiftNode<'_>) -> bool {
    match node.kind().as_ref() {
        "key_path_expression" => true,
        "navigation_expression" => node
            .field("target")
            .is_some_and(|target| is_key_path_chain(&target)),
        _ => false,
    }
}

/// True when `node` is an arbitrarily deep type-qualified path such as `Status`, `Payment.Status`,
/// or `A.B.C` where every segment starts with an ASCII uppercase letter.
pub(super) fn is_type_path_expr(node: &SwiftNode<'_>) -> bool {
    match node.kind().as_ref() {
        "simple_identifier" => syntax::is_uppercase_ident(node.text().trim()),
        "navigation_expression" => {
            let Some(target) = node.field("target") else {
                return false;
            };
            let Some(member) = syntax::extract_navigation_member(node) else {
                return false;
            };
            syntax::is_uppercase_ident(&member) && is_type_path_expr(&target)
        }
        _ => false,
    }
}

fn collect_bound_names_iter<'a>(
    iter: impl Iterator<Item = SwiftNode<'a>>,
    under_binding: bool,
    out: &mut Vec<String>,
) {
    let mut binding_active = under_binding;
    let mut children = iter.peekable();
    while let Some(child) = children.next() {
        if child.kind() == "value_binding_pattern" {
            binding_active = true;
            continue;
        }
        if !child.is_named() && child.text().trim() == "." {
            let _ = children.next();
            continue;
        }
        if child.kind() == "simple_identifier"
            && children
                .peek()
                .is_some_and(|n| !n.is_named() && n.text().trim() == ":")
        {
            let _ = children.next();
            continue;
        }
        collect_bound_names_node(&child, binding_active, out);
    }
}

fn collect_bound_names_node(node: &SwiftNode<'_>, under_binding: bool, out: &mut Vec<String>) {
    if node.kind() == "user_type" {
        return;
    }
    if node.kind() == "simple_identifier" {
        if under_binding {
            let text = node.text();
            if !text.is_empty() && text.as_ref() != "_" {
                out.push(text.into_owned());
            }
        }
        return;
    }
    if let Some(bound) = node.field("bound_identifier") {
        let text = bound.text();
        if !text.is_empty() && text.as_ref() != "_" {
            out.push(text.into_owned());
        }
        return;
    }
    collect_bound_names_iter(node.children(), under_binding, out);
}

pub(super) fn unwrap_expr_child<'a>(node: &SwiftNode<'a>) -> Option<SwiftNode<'a>> {
    node.field("target")
        .or_else(|| node.field("expr"))
        .or_else(|| node.children().find(SwiftNode::is_named))
}

pub(super) fn unwrap_transparent_callee<'a>(node: &SwiftNode<'a>) -> SwiftNode<'a> {
    let mut current = node.clone();
    loop {
        if let Some(inner) = syntax::unwrap_parenthesized_tuple(&current) {
            current = inner;
            continue;
        }
        if matches!(
            current.kind().as_ref(),
            "postfix_expression" | "try_expression" | "await_expression"
        ) && let Some(inner) = unwrap_expr_child(&current)
        {
            current = inner;
            continue;
        }
        break;
    }
    current
}

pub(super) fn extract_qualified_type_path(nav: &SwiftNode<'_>) -> Option<String> {
    let target = nav.field("target")?;
    let member = syntax::extract_navigation_member(nav)?;
    if !syntax::is_uppercase_ident(&member) {
        return None;
    }
    match target.kind().as_ref() {
        "simple_identifier" => {
            let root = target.text();
            let root = root.trim();
            syntax::is_uppercase_ident(root).then(|| format!("{root}::{member}"))
        }
        "navigation_expression" => {
            let prefix = extract_qualified_type_path(&target)?;
            Some(format!("{prefix}::{member}"))
        }
        _ => None,
    }
}

pub(super) fn infer_expr_origin(
    node: &SwiftNode<'_>,
    transparent_wrappers: &[String],
) -> (Option<TypeRef>, Option<String>) {
    if let Some(inner) = syntax::unwrap_parenthesized_tuple(node) {
        return infer_expr_origin(&inner, transparent_wrappers);
    }
    match node.kind().as_ref() {
        "postfix_expression" | "try_expression" | "await_expression" => unwrap_expr_child(node)
            .map_or((None, None), |inner| {
                infer_expr_origin(&inner, transparent_wrappers)
            }),
        "as_expression" => node
            .field("type")
            .and_then(|t| syntax::extract_type_ref(&t, transparent_wrappers))
            .map_or_else(
                || {
                    node.field("expr").map_or((None, None), |e| {
                        infer_expr_origin(&e, transparent_wrappers)
                    })
                },
                |ty| (Some(ty), None),
            ),
        "constructor_expression" => (
            node.field("constructed_type")
                .and_then(|t| syntax::extract_nominal_type_ref(&t)),
            None,
        ),
        "call_expression" if !is_subscript_call(node) => {
            let Some(raw_callee) = node.children().find(SwiftNode::is_named) else {
                return (None, None);
            };
            let callee = unwrap_transparent_callee(&raw_callee);
            let callee_kind = callee.kind();
            if let Some(ty) = syntax::shorthand_collection_type(callee_kind.as_ref()) {
                return (Some(TypeRef::Named(String::from(ty))), None);
            }
            match callee_kind.as_ref() {
                "simple_identifier" => {
                    let name = callee.text().into_owned();
                    if name == "Self" {
                        (Some(TypeRef::SelfType), None)
                    } else if syntax::is_uppercase_ident(&name) {
                        (Some(TypeRef::Named(name)), None)
                    } else {
                        (None, Some(name))
                    }
                }
                "navigation_expression" => {
                    let Some(target) = callee.field("target") else {
                        return (None, None);
                    };
                    let Some(member) = syntax::extract_navigation_member(&callee) else {
                        return (None, None);
                    };
                    if target.kind() == "self_expression" {
                        return (None, Some(member));
                    }
                    if target.kind() == "simple_identifier" {
                        let target_text = target.text();
                        if target_text == "Self" {
                            return (Some(TypeRef::SelfType), None);
                        }
                        if syntax::is_uppercase_ident(target_text.as_ref()) {
                            let ty = if syntax::is_uppercase_ident(&member) {
                                format!("{target_text}::{member}")
                            } else {
                                target_text.into_owned()
                            };
                            return (Some(TypeRef::Named(ty)), None);
                        }
                    }
                    if target.kind() == "navigation_expression"
                        && let Some(qualified) = extract_qualified_type_path(&target)
                    {
                        let ty = if syntax::is_uppercase_ident(&member) {
                            format!("{qualified}::{member}")
                        } else {
                            qualified
                        };
                        return (Some(TypeRef::Named(ty)), None);
                    }
                    (None, None)
                }
                _ => (None, None),
            }
        }
        _ => (None, None),
    }
}
