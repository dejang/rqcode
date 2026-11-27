use super::TsNode;
use crate::ast::item::{CallKind, CallReceiver, CallSite, TypeRef};
use crate::ast::{ModulePath, SyntaxTree};
use crate::diagnostic::Span;
use crate::lang::common::{BodyAccumulator, is_comment_kind};
use std::collections::HashMap;

/// Matches TypeScript's `isIntrinsicJsxName`: lowercase start or hyphenated custom element.
pub(super) fn is_intrinsic_jsx_tag(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase()) || name.contains('-')
}

/// Outcome of dispatching a CST node through [`TsJsxWalker`].
pub(super) enum JsxWalkResult<'a> {
    NotJsx,
    Handled(Vec<TsNode<'a>>),
}

/// Walks JSX element nodes, recording component/member calls and returning nested
/// attribute/expression children for [`super::body::TsBodyWalker`] to traverse.
pub(super) struct TsJsxWalker<'a, 'b, 'c> {
    tree: &'a SyntaxTree,
    namespace_imports: &'b HashMap<String, ModulePath>,
    acc: &'c mut BodyAccumulator,
}

impl<'a, 'b, 'c> TsJsxWalker<'a, 'b, 'c> {
    pub(super) fn new(
        tree: &'a SyntaxTree,
        namespace_imports: &'b HashMap<String, ModulePath>,
        acc: &'c mut BodyAccumulator,
    ) -> Self {
        Self {
            tree,
            namespace_imports,
            acc,
        }
    }

    pub(super) fn walk_jsx_node(&mut self, node: &TsNode<'a>) -> JsxWalkResult<'a> {
        match node.kind().as_ref() {
            "jsx_closing_element" | "jsx_text" => JsxWalkResult::Handled(Vec::new()),
            "jsx_opening_element" | "jsx_self_closing_element" => {
                let mut pending = Vec::new();
                if let Some(name_node) = node.field("name") {
                    let span = self.tree.node_span(node);
                    match name_node.kind().as_ref() {
                        "identifier" => {
                            let tag = name_node.text();
                            if !is_intrinsic_jsx_tag(tag.as_ref()) {
                                let tag_name = tag.into_owned();
                                self.acc.push_call(CallSite {
                                    kind: CallKind::Free,
                                    callee: tag_name.clone(),
                                    receiver: None,
                                    arg_receivers: Vec::new(),
                                    span,
                                });
                                self.acc.push_referenced_ident(tag_name.clone());
                                self.acc.push_constructed_type(TypeRef::Named(tag_name));
                            }
                        }
                        "member_expression" | "nested_identifier" => {
                            self.record_jsx_member_call(&name_node, span, &mut pending);
                        }
                        _ => {}
                    }
                }
                for child in node.children() {
                    if matches!(child.kind().as_ref(), "jsx_attribute" | "jsx_expression") {
                        pending.push(child);
                    }
                }
                JsxWalkResult::Handled(pending)
            }
            _ => JsxWalkResult::NotJsx,
        }
    }

    fn record_jsx_member_call(
        &mut self,
        name_node: &TsNode<'a>,
        span: Span,
        pending: &mut Vec<TsNode<'a>>,
    ) {
        let Some((obj_node, prop_node)) = split_jsx_member(name_node) else {
            return;
        };
        let prop_name = prop_node.text().into_owned();
        if prop_name.is_empty() {
            return;
        }
        let mut cur = obj_node.clone();
        while matches!(
            cur.kind().as_ref(),
            "member_expression" | "nested_identifier"
        ) {
            let Some((next_obj, _)) = split_jsx_member(&cur) else {
                break;
            };
            cur = next_obj;
        }
        if cur.kind() == "identifier" {
            let root = cur.text().into_owned();
            let (kind, receiver) = if let Some(ns_mod) = self.namespace_imports.get(&root) {
                (CallKind::Scoped, CallReceiver::Named(ns_mod.to_string()))
            } else if root.chars().next().is_some_and(|c| c.is_ascii_uppercase()) {
                (CallKind::Scoped, CallReceiver::Named(root))
            } else {
                (CallKind::Method, CallReceiver::Named(root))
            };
            self.acc.push_call(CallSite {
                kind,
                callee: prop_name.clone(),
                receiver: Some(receiver),
                arg_receivers: Vec::new(),
                span,
            });
        }
        self.acc.push_scoped_ident_ref(prop_name);
        self.walk_jsx_member_object(&obj_node, pending);
    }

    /// Walk the receiver object of a JSX member tag without recording a `FieldAccess`.
    fn walk_jsx_member_object(&mut self, node: &TsNode<'a>, pending: &mut Vec<TsNode<'a>>) {
        match node.kind().as_ref() {
            "member_expression" | "nested_identifier" => {
                if let Some((obj, prop)) = split_jsx_member(node) {
                    let prop_text = prop.text().into_owned();
                    if !prop_text.is_empty() {
                        self.acc.push_scoped_ident_ref(prop_text);
                    }
                    self.walk_jsx_member_object(&obj, pending);
                }
            }
            "identifier" => {
                self.acc.push_referenced_ident(node.text().into_owned());
            }
            _ => pending.push(node.clone()),
        }
    }
}

fn split_jsx_member<'a>(node: &TsNode<'a>) -> Option<(TsNode<'a>, TsNode<'a>)> {
    if let (Some(obj), Some(prop)) = (node.field("object"), node.field("property")) {
        return Some((obj, prop));
    }
    let mut named = node
        .children()
        .filter(|c| c.is_named() && !is_comment_kind(c.kind().as_ref()));
    let obj = named.next()?;
    let prop = named.last()?;
    Some((obj, prop))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::LanguageFrontend;
    use crate::lang::typescript::{TypeScriptFrontend, tsx_language_spec};

    #[test]
    fn classifies_intrinsic_html_and_custom_element_tags() {
        assert!(is_intrinsic_jsx_tag("div"));
        assert!(is_intrinsic_jsx_tag("span"));
        assert!(is_intrinsic_jsx_tag("my-widget"));
        assert!(is_intrinsic_jsx_tag("x-card"));
        assert!(!is_intrinsic_jsx_tag("OrderCard"));
        assert!(!is_intrinsic_jsx_tag("_Internal"));
        assert!(!is_intrinsic_jsx_tag("$Button"));
    }

    #[test]
    fn jsx_walker_records_component_and_member_calls_without_intrinsic_tags() {
        let src = r#"
import * as UI from "./ui";
export function Page() {
    return (
        <div className="wrap">
            <my-banner />
            <svg:path />
            <OrderCard />
            <UI.Badge />
            <Dialog.Root />
        </div>
    );
}
"#;
        let tree = SyntaxTree::parse("src/Page.tsx", "src/Page.tsx", tsx_language_spec(), src);
        let ast = TypeScriptFrontend::new().extract(&tree, std::path::Path::new("."));
        let page = ast
            .functions
            .iter()
            .find(|f| f.name == "Page")
            .expect("Page extracted");

        assert!(
            page.calls
                .iter()
                .any(|c| c.kind == CallKind::Free && c.callee == "OrderCard")
        );
        assert!(page.calls.iter().any(|c| {
            c.kind == CallKind::Scoped
                && c.callee == "Badge"
                && c.receiver == Some(CallReceiver::Named("src::ui".into()))
        }));
        assert!(page.calls.iter().any(|c| {
            c.kind == CallKind::Scoped
                && c.callee == "Root"
                && c.receiver == Some(CallReceiver::Named("Dialog".into()))
        }));
        assert!(
            !page
                .calls
                .iter()
                .any(|c| matches!(c.callee.as_str(), "div" | "my-banner" | "path"))
        );
        assert_eq!(page.field_accesses.len(), 0);
        assert!(
            !page
                .constructed_types
                .iter()
                .any(|t| *t == TypeRef::Named("Dialog".into()))
        );
    }
}
