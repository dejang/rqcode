use super::TsNode;
use super::macro_tokens::{is_delimited_tree, skip_turbofish, starts_ascii_upper};
use super::syntax;
use crate::ast::similarity::{AstKind, LitKind, NameSpace};
use crate::lang::common::{CstNormalizeDialect, CstNormalizer};

/// Stage 1: Rust CST-to-[`crate::ast::similarity::NormalizedTree`] dialect.
#[derive(Default)]
pub(super) struct RustNormalizeDialect;

impl RustNormalizeDialect {
    fn emit_scoped_path(
        &self,
        norm: &mut CstNormalizer,
        node: &TsNode<'_>,
        uppercase_owner_only: bool,
    ) -> bool {
        let _ = self;
        let Some((prefix, name)) = syntax::split_scoped_path(node) else {
            return false;
        };
        if let Some(owner) = prefix.last()
            && (!uppercase_owner_only || starts_ascii_upper(owner.as_str()))
        {
            norm.tb().exact(owner.as_str());
        }
        norm.tb().exact(name);
        true
    }

    fn emit_macro_call(
        &mut self,
        norm: &mut CstNormalizer,
        owner: Option<&str>,
        callee: &str,
        args: &TsNode<'_>,
    ) {
        let mark = norm.tb().enter(AstKind::Call);
        if let Some(owner) = owner {
            norm.tb().exact(owner);
        }
        norm.tb().exact(callee);
        self.walk_macro_token_tree(norm, args);
        norm.tb().exit(mark);
    }

    fn walk_macro_token_tree(&mut self, norm: &mut CstNormalizer, tt: &TsNode<'_>) {
        norm.with_depth(|norm| {
            let tokens: Vec<TsNode<'_>> = tt
                .children()
                .filter(|c| !matches!(c.kind().as_ref(), "line_comment" | "block_comment"))
                .collect();
            let mut i = 0;
            while i < tokens.len() {
                let tok = &tokens[i];
                let kind = tok.kind();
                if kind == "token_tree" {
                    self.walk_macro_token_tree(norm, tok);
                    i += 1;
                    continue;
                }
                if tok.text() == "." && i + 1 < tokens.len() && tokens[i + 1].kind() == "identifier"
                {
                    let member = tokens[i + 1].text();
                    let after_tf = skip_turbofish(&tokens, i + 2);
                    if after_tf < tokens.len() && is_delimited_tree(&tokens[after_tf], "(") {
                        self.emit_macro_call(norm, None, member.trim(), &tokens[after_tf]);
                        i = after_tf + 1;
                    } else {
                        let mark = norm.tb().enter(AstKind::Field);
                        norm.tb().symbol(NameSpace::Member, member.trim());
                        norm.tb().exit(mark);
                        i += 2;
                    }
                    continue;
                }
                if tok.text() == "::" && i + 1 < tokens.len() && is_path_ident(&tokens[i + 1]) {
                    i += 1;
                    continue;
                }
                if is_path_ident(tok) {
                    i = self.walk_macro_ident(norm, &tokens, i);
                    continue;
                }
                norm.walk(self, tok);
                i += 1;
            }
        });
    }

    fn walk_macro_ident(
        &mut self,
        norm: &mut CstNormalizer,
        tokens: &[TsNode<'_>],
        start: usize,
    ) -> usize {
        // Nested macro call: `foo!(...)` / `foo![...]` / `foo!{...}`.
        if start + 2 < tokens.len()
            && tokens[start + 1].text() == "!"
            && tokens[start + 2].kind() == "token_tree"
        {
            let name = format!("{}!", tokens[start].text().trim());
            self.emit_macro_call(norm, None, &name, &tokens[start + 2]);
            return start + 3;
        }

        // Scoped path: `Status::Active`, `Vec::<T>::new(...)`, `std::matches!(...)`, `S::V { .. }`.
        let mut idx = start + 1;
        let mut last_owner: Option<String> = None;
        let mut last_name = tokens[start].text().trim().to_string();
        let mut is_scoped = false;
        loop {
            let after_tf = skip_turbofish(tokens, idx);
            if after_tf + 1 < tokens.len()
                && tokens[after_tf].text() == "::"
                && is_path_ident(&tokens[after_tf + 1])
            {
                last_owner = Some(std::mem::replace(
                    &mut last_name,
                    tokens[after_tf + 1].text().trim().to_string(),
                ));
                is_scoped = true;
                idx = after_tf + 2;
            } else {
                break;
            }
        }

        let after_tf = skip_turbofish(tokens, idx);
        if is_scoped {
            if idx + 1 < tokens.len()
                && tokens[idx].text() == "!"
                && tokens[idx + 1].kind() == "token_tree"
            {
                self.emit_macro_call(norm, None, &format!("{last_name}!"), &tokens[idx + 1]);
                return idx + 2;
            }
            if after_tf < tokens.len() && is_delimited_tree(&tokens[after_tf], "(") {
                let owner = last_owner.as_deref().filter(|o| starts_ascii_upper(o));
                self.emit_macro_call(norm, owner, &last_name, &tokens[after_tf]);
                return after_tf + 1;
            }
            if after_tf < tokens.len()
                && is_delimited_tree(&tokens[after_tf], "{")
                && starts_ascii_upper(&last_name)
            {
                let mark = norm.tb().enter(AstKind::Construct);
                if let Some(owner) = last_owner.as_deref() {
                    norm.tb().exact(owner);
                }
                norm.tb().exact(last_name);
                self.walk_macro_token_tree(norm, &tokens[after_tf]);
                norm.tb().exit(mark);
                return after_tf + 1;
            }
            let mark = norm.tb().enter(AstKind::Field);
            if let Some(owner) = last_owner.as_deref() {
                norm.tb().exact(owner);
            }
            norm.tb().exact(last_name);
            norm.tb().exit(mark);
            return idx;
        }

        // Unscoped call: `helper(a, b)` or `helper::<T>(a, b)`.
        if after_tf < tokens.len() && is_delimited_tree(&tokens[after_tf], "(") {
            self.emit_macro_call(norm, None, &last_name, &tokens[after_tf]);
            return after_tf + 1;
        }

        // Unscoped struct literal: `Order { total: 10 }` or `Order::<T> { total: 10 }`.
        if after_tf < tokens.len()
            && is_delimited_tree(&tokens[after_tf], "{")
            && starts_ascii_upper(&last_name)
        {
            let mark = norm.tb().enter(AstKind::Construct);
            norm.tb().exact(last_name);
            self.walk_macro_token_tree(norm, &tokens[after_tf]);
            norm.tb().exit(mark);
            return after_tf + 1;
        }

        if tokens[start].kind() == "self" {
            norm.tb().symbol(NameSpace::Plain, "self");
        } else if tokens[start].kind() == "primitive_type" || starts_ascii_upper(&last_name) {
            norm.tb().exact(last_name);
        } else {
            norm.tb().symbol(NameSpace::Plain, last_name);
        }
        start + 1
    }
}

fn is_path_ident(node: &TsNode<'_>) -> bool {
    matches!(
        node.kind().as_ref(),
        "identifier" | "primitive_type" | "self" | "super" | "crate"
    )
}

impl CstNormalizeDialect for RustNormalizeDialect {
    fn visit_compound(&mut self, norm: &mut CstNormalizer, node: &TsNode<'_>) -> bool {
        match node.kind().as_ref() {
            "line_comment" | "block_comment" | "attribute_item" | "type_arguments"
            | "type_parameters" => true,
            "string_literal" | "raw_string_literal" => {
                norm.tb().literal(LitKind::String);
                true
            }
            "char_literal" => {
                norm.tb().literal(LitKind::Char);
                true
            }
            "boolean_literal" => {
                norm.tb().literal(LitKind::Bool);
                true
            }
            "parenthesized_expression" => {
                norm.walk_named_children(self, node);
                true
            }
            "block" => norm.wrap_children(self, AstKind::Block, node),
            "if_expression" => norm.wrap_children(self, AstKind::If, node),
            "match_expression" => norm.wrap_children(self, AstKind::Branch, node),
            "match_arm" => norm.wrap_children(self, AstKind::BranchArm, node),
            "for_expression" | "while_expression" | "loop_expression" => {
                norm.wrap_children(self, AstKind::Loop, node)
            }
            "try_expression" => norm.wrap_children(self, AstKind::Try, node),
            "index_expression" => norm.wrap_children(self, AstKind::Index, node),
            "array_expression" | "tuple_expression" => {
                norm.wrap_children(self, AstKind::Collection, node)
            }
            "assignment_expression" | "compound_assignment_expr" => {
                norm.walk_assignment(self, node, "left", "right");
                true
            }
            "binary_expression" => {
                norm.walk_binary(self, node, "left", "right");
                true
            }
            "await_expression" => {
                norm.walk_tagged_named_children(self, AstKind::Unary, "await", node);
                true
            }
            "unary_expression" => {
                norm.walk_unary_fields(self, node, "operator", "value");
                true
            }
            "return_expression" => {
                norm.walk_tagged_named_children(self, AstKind::Return, "return", node);
                true
            }
            "break_expression" => {
                norm.walk_tagged_named_children(self, AstKind::Return, "break", node);
                true
            }
            "continue_expression" => {
                norm.walk_tagged_named_children(self, AstKind::Return, "continue", node);
                true
            }
            "field_expression" => {
                norm.walk_member_access(self, node, "value", "field");
                true
            }
            "call_expression" => {
                norm.walk_call(self, node, "function", "arguments");
                true
            }
            "scoped_identifier" | "scoped_type_identifier" => {
                let mark = norm.tb().enter(AstKind::Field);
                if !self.emit_scoped_path(norm, node, false) {
                    norm.emit_leaf_fallback(node);
                }
                norm.tb().exit(mark);
                true
            }
            "let_declaration" => {
                let mark = norm.tb().enter(AstKind::Bind);
                if let Some(pat) = node.field("pattern") {
                    norm.walk(self, &pat);
                }
                if let Some(val) = node.field("value") {
                    norm.walk(self, &val);
                }
                if let Some(alt) = node.field("alternative") {
                    let branch = norm.tb().enter(AstKind::If);
                    norm.walk(self, &alt);
                    norm.tb().exit(branch);
                }
                norm.tb().exit(mark);
                true
            }
            _ => false,
        }
    }

    fn visit_special(&mut self, norm: &mut CstNormalizer, node: &TsNode<'_>) -> bool {
        match node.kind().as_ref() {
            "tuple_struct_pattern" => {
                let mark = norm.tb().enter(AstKind::Construct);
                let ty_range = node.field("type").map(|ty_node| {
                    if !self.emit_scoped_path(norm, &ty_node, false) {
                        norm.tb().exact(ty_node.text().trim());
                    }
                    ty_node.range()
                });
                norm.walk_named_except(self, node, ty_range.as_ref());
                norm.tb().exit(mark);
                true
            }
            "reference_expression" => {
                let mark = norm.tb().enter(AstKind::Unary);
                let is_mut = node.children().any(|c| c.kind() == "mutable_specifier");
                norm.tb().exact(if is_mut { "&mut" } else { "&" });
                if let Some(val) = node.field("value") {
                    norm.walk(self, &val);
                }
                norm.tb().exit(mark);
                true
            }
            "macro_invocation" => {
                let mark = norm.tb().enter(AstKind::Call);
                if let Some(mac) = node.field("macro") {
                    let name = mac.text();
                    let tail = name.rsplit("::").next().unwrap_or(name.as_ref()).trim();
                    if !tail.is_empty() {
                        norm.tb().exact(format!("{tail}!"));
                    }
                }
                if let Some(tt) = node.children().find(|c| c.kind() == "token_tree") {
                    self.walk_macro_token_tree(norm, &tt);
                }
                norm.tb().exit(mark);
                true
            }
            "struct_expression" => {
                let mark = norm.tb().enter(AstKind::Construct);
                if let Some(name_node) = node.field("name") {
                    let target = if name_node.kind() == "generic_type" {
                        name_node.field("type").unwrap_or(name_node)
                    } else {
                        name_node
                    };
                    if !self.emit_scoped_path(norm, &target, false) {
                        let raw = target.text();
                        let ty = raw.split("::<").next().unwrap_or(raw.as_ref()).trim();
                        norm.tb().exact(ty);
                    }
                }
                if let Some(body) = node.field("body") {
                    for child in body.children() {
                        match child.kind().as_ref() {
                            "field_initializer" => {
                                let field = child.field("field");
                                let val = child.field("value");
                                norm.walk_labeled_arg(
                                    self,
                                    field
                                        .as_ref()
                                        .map(|f| f.text().trim().to_owned())
                                        .as_deref(),
                                    val.as_ref(),
                                );
                            }
                            "shorthand_field_initializer" => {
                                norm.emit_shorthand_arg(child.text().trim());
                            }
                            "base_field_initializer" => norm.walk(self, &child),
                            _ => {}
                        }
                    }
                }
                norm.tb().exit(mark);
                true
            }
            "closure_expression" => {
                let mark = norm.tb().enter(AstKind::Closure);
                if let Some(params) = node.field("parameters") {
                    for child in params.children() {
                        if child.kind() == "parameter" {
                            if let Some(pat) = child.field("pattern") {
                                norm.walk(self, &pat);
                            }
                        } else if child.is_named() {
                            norm.walk(self, &child);
                        }
                    }
                }
                if let Some(body) = node.field("body") {
                    norm.walk(self, &body);
                }
                norm.tb().exit(mark);
                true
            }
            _ => false,
        }
    }

    fn visit_callee(&mut self, norm: &mut CstNormalizer, node: &TsNode<'_>) {
        match node.kind().as_ref() {
            "identifier" => norm.tb().exact(node.text().trim()),
            "field_expression" => {
                if let Some(val) = node.field("value") {
                    norm.walk(self, &val);
                }
                if let Some(method) = node.field("field") {
                    norm.tb().exact(method.text().trim());
                }
            }
            "scoped_identifier" => {
                if !self.emit_scoped_path(norm, node, true) {
                    norm.emit_leaf_fallback(node);
                }
            }
            "generic_function" => {
                if let Some(inner) = node.field("function") {
                    self.visit_callee(norm, &inner);
                }
            }
            _ => norm.walk(self, node),
        }
    }

    fn visit_leaf(&mut self, norm: &mut CstNormalizer, node: &TsNode<'_>) {
        match node.kind().as_ref() {
            "identifier" => {
                let text = node.text();
                let trimmed = text.trim();
                if starts_ascii_upper(trimmed) {
                    norm.tb().exact(trimmed);
                } else {
                    norm.tb().symbol(NameSpace::Plain, trimmed);
                }
            }
            "self" => norm.tb().symbol(NameSpace::Plain, "self"),
            "field_identifier" | "shorthand_field_identifier" => {
                norm.tb().symbol(NameSpace::Member, node.text().trim());
            }
            "type_identifier" => norm.tb().symbol(NameSpace::Type, node.text().trim()),
            "integer_literal" | "float_literal" => norm.tb().literal(LitKind::Number),
            "true" | "false" => norm.tb().literal(LitKind::Bool),
            "lifetime" | "(" | ")" | "{" | "}" | "[" | "]" | ";" | "," | "." | ":" | "::"
            | "\"" | "'" | "|" | "=>" | "->" | "let" | "mut" | "ref" | "in" | "if" | "else"
            | "match" | "for" | "while" | "loop" | "return" | "break" | "continue" | "move"
            | "async" | "fn" | "?" => {}
            _ => norm.emit_leaf_fallback(node),
        }
    }
}
