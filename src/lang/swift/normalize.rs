use super::SwiftNode;
use super::expr::{self, SwiftCondition, SwiftConditionalParts};
use super::syntax;
use crate::ast::similarity::{AstKind, LitKind, NameSpace};
use crate::lang::common::{CstNormalizeDialect, CstNormalizer};
use std::collections::HashSet;

/// Stage 1: Swift CST-to-[`crate::ast::similarity::NormalizedTree`] dialect.
///
/// Tracks enclosing owner members and lexically shadowed names so unshadowed implicit-self
/// property references (`total`) and method calls (`reset()`) normalize identically to explicit
/// `self.total` and `self.reset()`.
pub(super) struct SwiftNormalizeDialect<'a> {
    owner_methods: Option<&'a HashSet<String>>,
    owner_properties: Option<&'a HashSet<String>>,
    is_static: bool,
    shadowed: HashSet<String>,
}

impl<'a> SwiftNormalizeDialect<'a> {
    pub(super) fn new(
        owner_methods: Option<&'a HashSet<String>>,
        owner_properties: Option<&'a HashSet<String>>,
        is_static: bool,
        param_names: &[String],
    ) -> Self {
        Self {
            owner_methods,
            owner_properties,
            is_static,
            shadowed: param_names.iter().cloned().collect(),
        }
    }

    fn is_implicit_self_member(&self, check_methods: bool, name: &str) -> bool {
        let members = if check_methods {
            self.owner_methods
        } else {
            self.owner_properties
        };
        !self.is_static
            && members.is_some_and(|set| set.contains(name))
            && !self.shadowed.contains(name)
    }

    fn shadow_names(&mut self, names: &[String]) -> Vec<String> {
        let mut newly_added = Vec::new();
        for name in names {
            if self.shadowed.insert(name.clone()) {
                newly_added.push(name.clone());
            }
        }
        newly_added
    }

    fn unshadow_names(&mut self, names: &[String]) {
        for name in names {
            self.shadowed.remove(name);
        }
    }

    fn wrap_scoped(
        &mut self,
        norm: &mut CstNormalizer,
        kind: AstKind,
        node: &SwiftNode<'_>,
        bindings: &[String],
    ) {
        let added = self.shadow_names(bindings);
        norm.wrap_children(self, kind, node);
        self.unshadow_names(&added);
    }

    fn walk_labeled_children(&mut self, norm: &mut CstNormalizer, node: &SwiftNode<'_>) {
        let children: Vec<SwiftNode<'_>> = node.children().collect();
        let mut i = 0;
        while i < children.len() {
            if children[i].kind() == "simple_identifier"
                && children
                    .get(i + 1)
                    .is_some_and(|n| !n.is_named() && n.text().trim() == ":")
            {
                norm.tb()
                    .symbol(NameSpace::Label, children[i].text().trim());
                i += 2;
                continue;
            }
            norm.walk(self, &children[i]);
            i += 1;
        }
    }

    fn walk_pattern_slice(&mut self, norm: &mut CstNormalizer, nodes: &[SwiftNode<'_>]) {
        let mut i = 0;
        while i < nodes.len() {
            let child = &nodes[i];
            if child.kind() == "user_type"
                && nodes
                    .get(i + 1)
                    .is_some_and(|n| !n.is_named() && n.text().trim() == ".")
                && let Some(case_ident) =
                    nodes.get(i + 2).filter(|n| n.kind() == "simple_identifier")
            {
                let mark = norm.tb().enter(AstKind::Field);
                norm.tb().exact(child.text().trim());
                norm.tb().exact(case_ident.text().trim());
                norm.tb().exit(mark);
                i += 3;
                continue;
            }
            if !child.is_named()
                && child.text().trim() == "."
                && let Some(case_ident) =
                    nodes.get(i + 1).filter(|n| n.kind() == "simple_identifier")
            {
                norm.tb().exact(format!(".{}", case_ident.text().trim()));
                i += 2;
                continue;
            }
            if child.kind() == "simple_identifier"
                && nodes
                    .get(i + 1)
                    .is_some_and(|n| !n.is_named() && n.text().trim() == ":")
            {
                norm.tb().symbol(NameSpace::Label, child.text().trim());
                i += 2;
                continue;
            }
            if !matches!(
                child.kind().as_ref(),
                "value_binding_pattern" | "type_annotation" | "async"
            ) {
                norm.walk(self, child);
            }
            i += 1;
        }
    }

    fn walk_conditional(&mut self, norm: &mut CstNormalizer, node: &SwiftNode<'_>) {
        let kind = if node.kind() == "while_statement" {
            AstKind::Loop
        } else {
            AstKind::If
        };
        let mark = norm.tb().enter(kind);
        let parts = SwiftConditionalParts::parse(node);
        let saved = self.shadowed.clone();

        for cond in parts.conditions() {
            match cond {
                SwiftCondition::OptionalBinding {
                    pattern,
                    bound_names,
                    rhs,
                } => {
                    let bind = norm.tb().enter(AstKind::Bind);
                    let added = self.shadow_names(bound_names);
                    self.walk_pattern_slice(norm, pattern);
                    self.unshadow_names(&added);
                    if rhs.is_empty() {
                        self.walk_pattern_slice(norm, pattern);
                    } else {
                        for r in rhs {
                            norm.walk(self, r);
                        }
                    }
                    self.shadow_names(&added);
                    norm.tb().exit(bind);
                }
                SwiftCondition::CaseBinding {
                    pattern,
                    bound_names,
                    rhs,
                } => {
                    let bind = norm.tb().enter(AstKind::Bind);
                    let added = self.shadow_names(bound_names);
                    self.walk_pattern_slice(norm, pattern);
                    self.unshadow_names(&added);
                    for r in rhs {
                        norm.walk(self, r);
                    }
                    self.shadow_names(&added);
                    norm.tb().exit(bind);
                }
                SwiftCondition::Expr(exprs) => {
                    for e in exprs {
                        norm.walk(self, e);
                    }
                }
            }
        }

        if node.kind() == "guard_statement" {
            let after_guard = self.shadowed.clone();
            self.shadowed = saved;
            for child in parts.else_children() {
                norm.walk(self, child);
            }
            self.shadowed = after_guard;
        } else {
            for child in parts.then_children() {
                norm.walk(self, child);
            }
            self.shadowed = saved;
            for child in parts.else_children() {
                norm.walk(self, child);
            }
        }
        norm.tb().exit(mark);
    }

    fn walk_scoped_construct(&mut self, norm: &mut CstNormalizer, node: &SwiftNode<'_>) -> bool {
        match node.kind().as_ref() {
            "statements" => {
                let saved = self.shadowed.clone();
                norm.wrap_children(self, AstKind::Block, node);
                self.shadowed = saved;
            }
            "switch_entry" => {
                let vars = expr::extract_switch_entry_bindings(node);
                self.wrap_scoped(norm, AstKind::BranchArm, node, &vars);
            }
            "catch_block" => {
                let vars = expr::extract_catch_bindings(node);
                self.wrap_scoped(norm, AstKind::BranchArm, node, &vars);
            }
            "function_declaration" => {
                if let Some(name) = node.field("name") {
                    self.shadowed.insert(name.text().into_owned());
                }
                let nested_params = expr::extract_nested_func_params(node);
                let mark = norm.tb().enter(AstKind::Closure);
                for param in &nested_params {
                    norm.tb().symbol(NameSpace::Plain, param);
                }
                let added = self.shadow_names(&nested_params);
                if let Some(body) = node.field("body") {
                    norm.walk(self, &body);
                }
                self.unshadow_names(&added);
                norm.tb().exit(mark);
            }
            "property_declaration" => {
                for binding in syntax::extract_property_bindings(node) {
                    let mark = norm.tb().enter(AstKind::Bind);
                    let mut names = Vec::new();
                    expr::collect_pattern_names(&binding.pattern, &mut names);
                    let added = self.shadow_names(&names);
                    norm.walk(self, &binding.pattern);
                    self.unshadow_names(&added);
                    for expr_node in [
                        binding.value,
                        binding.computed_value,
                        binding.observer_block,
                    ]
                    .into_iter()
                    .flatten()
                    {
                        norm.walk(self, &expr_node);
                    }
                    self.shadow_names(&added);
                    norm.tb().exit(mark);
                }
            }
            "for_statement" => {
                let mut loop_vars = Vec::new();
                if let Some(item) = node.field("item") {
                    expr::collect_pattern_names(&item, &mut loop_vars);
                }
                let mark = norm.tb().enter(AstKind::Loop);
                let coll_range = node.field("collection").map(|coll| {
                    norm.walk(self, &coll);
                    coll.range()
                });
                let added = self.shadow_names(&loop_vars);
                for child in node.children() {
                    if coll_range != Some(child.range()) {
                        norm.walk(self, &child);
                    }
                }
                self.unshadow_names(&added);
                norm.tb().exit(mark);
            }
            "lambda_literal" => {
                let (capture_reads, mut closure_vars) =
                    expr::extract_closure_capture_bindings(node);
                for child in node
                    .children()
                    .filter(|c| c.kind() == "lambda_function_type")
                {
                    syntax::collect_leaf_idents(&child, &mut closure_vars);
                }
                let mark = norm.tb().enter(AstKind::Closure);
                for read in &capture_reads {
                    norm.walk(self, read);
                }
                let added = self.shadow_names(&closure_vars);
                for child in node.children() {
                    if child.kind() != "capture_list" {
                        norm.walk(self, &child);
                    }
                }
                self.unshadow_names(&added);
                norm.tb().exit(mark);
            }
            _ => return false,
        }
        true
    }
}

fn emit_type_path(norm: &mut CstNormalizer, node: &SwiftNode<'_>) {
    match node.kind().as_ref() {
        "simple_identifier" => norm.tb().exact(node.text().trim()),
        "navigation_expression" => {
            if let Some(target) = node.field("target") {
                emit_type_path(norm, &target);
            }
            if let Some(member) = syntax::extract_navigation_member(node) {
                norm.tb().exact(member);
            }
        }
        _ => {}
    }
}

fn implicit_member_name(node: &SwiftNode<'_>) -> Option<String> {
    if node.kind() != "prefix_expression" {
        return None;
    }
    let op = node.field("operation")?;
    if op.text().trim() != "." {
        return None;
    }
    let target = node.field("target")?;
    if target.kind() != "simple_identifier" {
        return None;
    }
    let trimmed = target.text();
    let trimmed = trimmed.trim();
    (!trimmed.is_empty()).then(|| format!(".{trimmed}"))
}

impl CstNormalizeDialect for SwiftNormalizeDialect<'_> {
    fn visit_compound(&mut self, norm: &mut CstNormalizer, node: &SwiftNode<'_>) -> bool {
        match node.kind().as_ref() {
            "comment"
            | "multiline_comment"
            | "type_annotation"
            | "type_arguments"
            | "type_parameters"
            | "class_declaration"
            | "protocol_declaration" => {}
            "self_expression" => norm.tb().symbol(NameSpace::Plain, "self"),
            "key_path_expression" => {
                let mark = norm.tb().enter(AstKind::Field);
                norm.tb().exact("\\");
                for child in node.children() {
                    match child.kind().as_ref() {
                        "type_identifier" | "user_type" => norm.tb().exact(child.text().trim()),
                        "simple_identifier" => {
                            norm.tb().symbol(NameSpace::Member, child.text().trim());
                        }
                        _ => {}
                    }
                }
                norm.tb().exit(mark);
            }
            "line_string_literal" | "multi_line_string_literal" => {
                norm.walk_interpolated_string(self, node, "interpolated_expression");
            }
            "raw_string_literal" => {
                norm.walk_interpolated_string(self, node, "raw_str_interpolation");
            }
            "regex_literal" => norm.tb().literal(LitKind::Other),
            "boolean_literal" => norm.tb().literal(LitKind::Bool),
            "nil_literal" => norm.tb().literal(LitKind::Null),
            "ternary_expression" => {
                norm.wrap_children(self, AstKind::If, node);
            }
            "if_statement" | "guard_statement" | "while_statement" => {
                self.walk_conditional(norm, node);
            }
            "switch_statement" => {
                norm.wrap_children(self, AstKind::Branch, node);
            }
            "pattern" => {
                let children: Vec<SwiftNode<'_>> = node.children().collect();
                self.walk_pattern_slice(norm, &children);
            }
            "repeat_while_statement" => {
                norm.wrap_children(self, AstKind::Loop, node);
            }
            "try_expression" | "do_statement" => {
                norm.wrap_children(self, AstKind::Try, node);
            }
            "array_literal" | "dictionary_literal" => {
                norm.wrap_children(self, AstKind::Collection, node);
            }
            "prefix_expression" => {
                if let Some(member) = implicit_member_name(node) {
                    norm.tb().exact(member);
                } else {
                    norm.walk_unary_fields(self, node, "operation", "target");
                }
            }
            "assignment" => norm.walk_assignment(self, node, "target", "result"),
            "additive_expression"
            | "multiplicative_expression"
            | "comparison_expression"
            | "equality_expression"
            | "conjunction_expression"
            | "disjunction_expression"
            | "nil_coalescing_expression"
            | "bitwise_operation"
            | "range_expression"
            | "infix_expression" => norm.walk_binary(self, node, "lhs", "rhs"),
            "await_expression" => {
                norm.walk_tagged_named_children(self, AstKind::Unary, "await", node);
            }
            "postfix_expression" => norm.walk_unary_fields(self, node, "operation", "target"),
            "control_transfer_statement" => {
                let kw = node
                    .children()
                    .next()
                    .map_or("return", |c| match c.text().trim() {
                        "break" => "break",
                        "continue" => "continue",
                        "throw" => "throw",
                        "fallthrough" => "fallthrough",
                        _ => "return",
                    });
                norm.walk_tagged_named_children(self, AstKind::Return, kw, node);
            }
            "value_argument_label" => norm.tb().symbol(NameSpace::Label, node.text().trim()),
            _ => return false,
        }
        true
    }

    fn visit_special(&mut self, norm: &mut CstNormalizer, node: &SwiftNode<'_>) -> bool {
        if self.walk_scoped_construct(norm, node) {
            return true;
        }
        match node.kind().as_ref() {
            "tuple_expression" => {
                if let Some(inner) = syntax::unwrap_parenthesized_tuple(node) {
                    norm.walk(self, &inner);
                } else {
                    let mark = norm.tb().enter(AstKind::Collection);
                    self.walk_labeled_children(norm, node);
                    norm.tb().exit(mark);
                }
            }
            "call_suffix" | "constructor_suffix" => self.walk_labeled_children(norm, node),
            "call_expression" => {
                if let Some(callee) = node.children().find(SwiftNode::is_named) {
                    let range = callee.range();
                    if expr::is_subscript_call(node) {
                        let mark = norm.tb().enter(AstKind::Index);
                        norm.walk(self, &callee);
                        norm.walk_named_except(self, node, Some(&range));
                        norm.tb().exit(mark);
                    } else {
                        let mark = norm.tb().enter(AstKind::Call);
                        self.visit_callee(norm, &callee);
                        norm.walk_named_except(self, node, Some(&range));
                        norm.tb().exit(mark);
                    }
                }
            }
            "constructor_expression" => {
                let mark = norm.tb().enter(AstKind::Construct);
                let ty_range = node.field("constructed_type").map(|ty_node| {
                    norm.tb().exact(ty_node.text().trim());
                    ty_node.range()
                });
                norm.walk_named_except(self, node, ty_range.as_ref());
                norm.tb().exit(mark);
            }
            "navigation_expression" => {
                let member = syntax::extract_navigation_member(node);
                if let Some(target) = node.field("target") {
                    let mark = norm.tb().enter(AstKind::Field);
                    if expr::is_type_path_expr(&target) {
                        emit_type_path(norm, &target);
                        if let Some(m) = member {
                            norm.tb().exact(m);
                        }
                    } else {
                        norm.walk(self, &target);
                        if let Some(m) = member {
                            norm.tb().symbol(NameSpace::Member, m);
                        }
                    }
                    norm.tb().exit(mark);
                }
            }
            "value_argument" => {
                if let Some(label_node) =
                    node.children().find(|c| c.kind() == "value_argument_label")
                {
                    let arg = norm.tb().enter(AstKind::Arg);
                    norm.tb().symbol(NameSpace::Label, label_node.text().trim());
                    let range = label_node.range();
                    norm.walk_named_except(self, node, Some(&range));
                    norm.tb().exit(arg);
                } else {
                    norm.walk_named_children(self, node);
                }
            }
            _ => return false,
        }
        true
    }

    fn visit_callee(&mut self, norm: &mut CstNormalizer, callee: &SwiftNode<'_>) {
        if let Some(member) = implicit_member_name(callee) {
            norm.tb().exact(member);
            return;
        }
        match callee.kind().as_ref() {
            "simple_identifier" => {
                let name = callee.text();
                let trimmed = name.trim();
                if self.is_implicit_self_member(true, trimmed)
                    || self.is_implicit_self_member(false, trimmed)
                {
                    norm.tb().symbol(NameSpace::Plain, "self");
                }
                norm.tb().exact(trimmed);
            }
            "navigation_expression" => {
                let member = syntax::extract_navigation_member(callee);
                if let Some(target) = callee.field("target") {
                    if expr::is_type_path_expr(&target) {
                        emit_type_path(norm, &target);
                    } else {
                        norm.walk(self, &target);
                    }
                    if let Some(m) = member {
                        norm.tb().exact(m);
                    }
                }
            }
            _ => norm.walk(self, callee),
        }
    }

    fn visit_leaf(&mut self, norm: &mut CstNormalizer, node: &SwiftNode<'_>) {
        match node.kind().as_ref() {
            "simple_identifier" => {
                let name = node.text();
                let trimmed = name.trim();
                if self.is_implicit_self_member(false, trimmed) {
                    let mark = norm.tb().enter(AstKind::Field);
                    norm.tb().symbol(NameSpace::Plain, "self");
                    norm.tb().symbol(NameSpace::Member, trimmed);
                    norm.tb().exit(mark);
                } else {
                    norm.tb().symbol(NameSpace::Plain, trimmed);
                }
            }
            "self_expression" => norm.tb().symbol(NameSpace::Plain, "self"),
            "type_identifier" => norm.tb().symbol(NameSpace::Type, node.text().trim()),
            "integer_literal" | "real_literal" | "hex_literal" | "oct_literal" | "bin_literal" => {
                norm.tb().literal(LitKind::Number);
            }
            "nil" => norm.tb().literal(LitKind::Null),
            "(" | ")" | "{" | "}" | "[" | "]" | ";" | "," | "." | ":" | "\"" | "->" | "let"
            | "var" | "in" | "if" | "else" | "guard" | "switch" | "case" | "default" | "for"
            | "while" | "repeat" | "return" | "throw" | "break" | "continue" | "do" | "catch"
            | "try" | "where" => {}
            _ => norm.emit_leaf_fallback(node),
        }
    }
}
