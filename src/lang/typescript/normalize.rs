use super::TsNode;
use crate::ast::similarity::{AstKind, LitKind, NameSpace};
use crate::lang::common::{CstNormalizeDialect, CstNormalizer, is_comment_kind};

/// Stage 1: TypeScript/TSX CST-to-[`crate::ast::similarity::NormalizedTree`] dialect.
#[derive(Default)]
pub(super) struct TsNormalizeDialect;

impl TsNormalizeDialect {
    fn visit_jsx(&mut self, norm: &mut CstNormalizer, node: &TsNode<'_>) -> bool {
        match node.kind().as_ref() {
            "jsx_element" => {
                let mark = norm.tb().enter(AstKind::Construct);
                let open_tag = node.field("open_tag");
                let close_tag = node.field("close_tag");
                let open_range = open_tag.as_ref().map(TsNode::range);
                let close_range = close_tag.as_ref().map(TsNode::range);

                if let Some(open) = open_tag {
                    if let Some(name) = open.field("name") {
                        norm.tb().exact(name.text().trim());
                    } else {
                        norm.tb().exact("<>");
                    }
                    self.walk_jsx_attributes(norm, &open);
                } else {
                    norm.tb().exact("<>");
                }

                for child in node.children() {
                    let range = Some(child.range());
                    if range == open_range
                        || range == close_range
                        || !child.is_named()
                        || is_comment_kind(child.kind().as_ref())
                        || child.kind() == "jsx_closing_element"
                    {
                        continue;
                    }
                    norm.walk(self, &child);
                }
                norm.tb().exit(mark);
                true
            }
            "jsx_self_closing_element" => {
                let mark = norm.tb().enter(AstKind::Construct);
                if let Some(name) = node.field("name") {
                    norm.tb().exact(name.text().trim());
                }
                self.walk_jsx_attributes(norm, node);
                norm.tb().exit(mark);
                true
            }
            "jsx_attribute" => {
                let mut named = node
                    .children()
                    .filter(|c| c.is_named() && !is_comment_kind(c.kind().as_ref()));
                let attr_name = named.next();
                let attr_val = named.next();
                let mark = norm.tb().enter(AstKind::Arg);
                if let Some(name) = attr_name {
                    norm.tb().exact(name.text().trim());
                }
                if let Some(val) = attr_val {
                    norm.walk(self, &val);
                } else {
                    norm.tb().literal(LitKind::Bool);
                }
                norm.tb().exit(mark);
                true
            }
            _ => false,
        }
    }

    fn walk_jsx_attributes(&mut self, norm: &mut CstNormalizer, tag: &TsNode<'_>) {
        for child in tag.children() {
            if matches!(child.kind().as_ref(), "jsx_attribute" | "jsx_expression") {
                norm.walk(self, &child);
            }
        }
    }
}

impl CstNormalizeDialect for TsNormalizeDialect {
    fn visit_compound(&mut self, norm: &mut CstNormalizer, node: &TsNode<'_>) -> bool {
        match node.kind().as_ref() {
            "comment"
            | "type_annotation"
            | "type_predicate_annotation"
            | "type_arguments"
            | "type_parameters"
            | "jsx_closing_element" => true,
            "jsx_text" => {
                if !node.text().trim().is_empty() {
                    norm.tb().literal(LitKind::String);
                }
                true
            }
            "html_character_reference" | "string" => {
                norm.tb().literal(LitKind::String);
                true
            }
            "jsx_expression" => {
                for child in node.children() {
                    if child.is_named() && !is_comment_kind(child.kind().as_ref()) {
                        norm.walk(self, &child);
                    }
                }
                true
            }
            "template_string" => {
                norm.walk_interpolated_string(self, node, "template_substitution");
                true
            }
            "regex" => {
                norm.tb().literal(LitKind::Other);
                true
            }
            "parenthesized_expression"
            | "non_null_expression"
            | "satisfies_expression"
            | "as_expression" => {
                if let Some(inner) = node
                    .children()
                    .find(|c| c.is_named() && !is_comment_kind(c.kind().as_ref()))
                {
                    norm.walk(self, &inner);
                }
                true
            }
            "statement_block" => norm.wrap_children(self, AstKind::Block, node),
            "if_statement" | "ternary_expression" => norm.wrap_children(self, AstKind::If, node),
            "switch_statement" => norm.wrap_children(self, AstKind::Branch, node),
            "switch_case" | "switch_default" => norm.wrap_children(self, AstKind::BranchArm, node),
            "for_statement" | "for_in_statement" | "while_statement" | "do_statement" => {
                norm.wrap_children(self, AstKind::Loop, node)
            }
            "try_statement" => norm.wrap_children(self, AstKind::Try, node),
            "arrow_function" | "function_expression" => {
                norm.wrap_children(self, AstKind::Closure, node)
            }
            "subscript_expression" => norm.wrap_children(self, AstKind::Index, node),
            "array" => norm.wrap_children(self, AstKind::Collection, node),
            "assignment_expression" | "augmented_assignment_expression" => {
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
                norm.walk_unary_fields(self, node, "operator", "argument");
                true
            }
            "return_statement" => {
                norm.walk_tagged_named_children(self, AstKind::Return, "return", node);
                true
            }
            "throw_statement" => {
                norm.walk_tagged_named_children(self, AstKind::Return, "throw", node);
                true
            }
            "break_statement" => {
                norm.walk_tagged_named_children(self, AstKind::Return, "break", node);
                true
            }
            "continue_statement" => {
                norm.walk_tagged_named_children(self, AstKind::Return, "continue", node);
                true
            }
            "call_expression" => {
                norm.walk_call(self, node, "function", "arguments");
                true
            }
            "variable_declarator" => {
                norm.walk_binding(self, node, "name", "value");
                true
            }
            _ => false,
        }
    }

    fn visit_special(&mut self, norm: &mut CstNormalizer, node: &TsNode<'_>) -> bool {
        if self.visit_jsx(norm, node) {
            return true;
        }
        match node.kind().as_ref() {
            "member_expression" => {
                if let Some(obj) = node.field("object")
                    && obj.kind() == "identifier"
                    && obj
                        .text()
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_uppercase())
                {
                    let mark = norm.tb().enter(AstKind::Field);
                    norm.tb().exact(obj.text().trim());
                    if let Some(prop) = node.field("property") {
                        norm.tb().exact(prop.text().trim());
                    }
                    norm.tb().exit(mark);
                } else {
                    norm.walk_member_access(self, node, "object", "property");
                }
                true
            }
            "update_expression" => {
                let mark = norm.tb().enter(AstKind::Assign);
                if let Some(op) = node.field("operator") {
                    norm.tb().exact(op.text().trim());
                }
                if let Some(arg) = node.field("argument") {
                    norm.walk(self, &arg);
                }
                norm.tb().exit(mark);
                true
            }
            "new_expression" => {
                let mark = norm.tb().enter(AstKind::Construct);
                if let Some(ctor) = node.field("constructor") {
                    norm.tb().exact(ctor.text().trim());
                }
                if let Some(args) = node.field("arguments") {
                    norm.walk_named_children(self, &args);
                }
                norm.tb().exit(mark);
                true
            }
            "object" => {
                let mark = norm.tb().enter(AstKind::Construct);
                for child in node.children() {
                    match child.kind().as_ref() {
                        "pair" => {
                            let key = child.field("key");
                            let val = child.field("value");
                            norm.walk_labeled_arg(
                                self,
                                key.as_ref().map(|k| k.text().trim().to_owned()).as_deref(),
                                val.as_ref(),
                            );
                        }
                        "shorthand_property_identifier" => {
                            norm.emit_shorthand_arg(child.text().trim());
                        }
                        _ if child.is_named() => norm.walk(self, &child),
                        _ => {}
                    }
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
            "member_expression" => {
                if let Some(obj) = node.field("object") {
                    if obj.kind() == "identifier"
                        && obj
                            .text()
                            .chars()
                            .next()
                            .is_some_and(|c| c.is_ascii_uppercase())
                    {
                        norm.tb().exact(obj.text().trim());
                    } else {
                        norm.walk(self, &obj);
                    }
                }
                if let Some(prop) = node.field("property") {
                    norm.tb().exact(prop.text().trim());
                }
            }
            _ => norm.walk(self, node),
        }
    }

    fn walk_call_args(&mut self, norm: &mut CstNormalizer, args: &TsNode<'_>) {
        if args.kind() == "template_string" {
            norm.walk(self, args);
        } else {
            norm.walk_named_children(self, args);
        }
    }

    fn visit_leaf(&mut self, norm: &mut CstNormalizer, node: &TsNode<'_>) {
        match node.kind().as_ref() {
            "identifier" => norm.tb().symbol(NameSpace::Plain, node.text().trim()),
            "this" => norm.tb().symbol(NameSpace::Plain, "self"),
            "property_identifier"
            | "private_property_identifier"
            | "shorthand_property_identifier"
            | "shorthand_property_identifier_pattern" => {
                norm.tb().symbol(NameSpace::Member, node.text().trim());
            }
            "type_identifier" => norm.tb().symbol(NameSpace::Type, node.text().trim()),
            "number" => norm.tb().literal(LitKind::Number),
            "string_fragment" => norm.tb().literal(LitKind::String),
            "true" | "false" => norm.tb().literal(LitKind::Bool),
            "null" | "undefined" => norm.tb().literal(LitKind::Null),
            "regex_pattern" => norm.tb().literal(LitKind::Other),
            "(" | ")" | "{" | "}" | "[" | "]" | ";" | "," | "." | ":" | "\"" | "'" | "`" | "?."
            | "=>" | "const" | "let" | "var" | "of" | "in" | "if" | "else" | "for" | "while"
            | "do" | "switch" | "case" | "default" | "return" | "throw" | "break" | "continue"
            | "try" | "catch" | "finally" | "new" | "function" | "async" => {}
            _ => norm.emit_leaf_fallback(node),
        }
    }
}
