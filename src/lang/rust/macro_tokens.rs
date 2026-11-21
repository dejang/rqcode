use super::TsNode;
use super::syntax;
use crate::ast::SyntaxTree;
use crate::ast::item::{CallKind, CallReceiver, CallSite, FieldAccess, FieldInit, TypeRef};

pub(super) struct MacroTokenData {
    pub(super) field_accesses: Vec<FieldAccess>,
    pub(super) calls: Vec<CallSite>,
    pub(super) constructed_types: Vec<TypeRef>,
    pub(super) field_inits: Vec<FieldInit>,
}

/// Scans unexpanded Rust macro `token_tree` arguments (e.g., `assert_eq!(o.field, helper())`)
/// to extract calls, field accesses, and struct literal initializers.
pub(super) struct MacroTokenScanner<'a> {
    tree: &'a SyntaxTree,
    field_accesses: Vec<FieldAccess>,
    calls: Vec<CallSite>,
    constructed_types: Vec<TypeRef>,
    field_inits: Vec<FieldInit>,
}

impl<'a> MacroTokenScanner<'a> {
    pub(super) fn new(tree: &'a SyntaxTree) -> Self {
        Self {
            tree,
            field_accesses: Vec::new(),
            calls: Vec::new(),
            constructed_types: Vec::new(),
            field_inits: Vec::new(),
        }
    }

    pub(super) fn scan_token_tree(mut self, tree_node: &TsNode<'a>) -> MacroTokenData {
        self.scan(tree_node);
        MacroTokenData {
            field_accesses: self.field_accesses,
            calls: self.calls,
            constructed_types: self.constructed_types,
            field_inits: self.field_inits,
        }
    }

    fn scan(&mut self, tree_node: &TsNode<'a>) {
        let nodes: Vec<TsNode<'a>> = tree_node
            .children()
            .filter(|c| !matches!(c.kind().as_ref(), "line_comment" | "block_comment"))
            .collect();

        let mut i = 0;
        while i < nodes.len() {
            let kind = nodes[i].kind();
            if kind == "token_tree" {
                self.scan(&nodes[i]);
                i += 1;
                continue;
            }
            if kind == "identifier" || kind == "self" {
                i = self.consume_ident(&nodes, i);
                continue;
            }
            i += 1;
        }
    }

    fn consume_ident(&mut self, nodes: &[TsNode<'a>], start: usize) -> usize {
        if start + 2 < nodes.len()
            && nodes[start + 1].text() == "::"
            && nodes[start + 2].kind() == "identifier"
        {
            return self.consume_scoped(nodes, start);
        }

        let ident = nodes[start].text().into_owned();

        if start + 1 < nodes.len()
            && is_delimited_tree(&nodes[start + 1], "{")
            && ident.chars().next().is_some_and(|c| c.is_ascii_uppercase())
        {
            self.record_struct_fields(&ident, &nodes[start + 1]);
            self.scan(&nodes[start + 1]);
            return start + 2;
        }

        if start + 2 < nodes.len()
            && nodes[start + 1].text() == "."
            && nodes[start + 2].kind() == "identifier"
        {
            return self.consume_dot_chain(nodes, start, &ident);
        }

        let after_tf = skip_turbofish(nodes, start + 1);
        if after_tf < nodes.len() && is_delimited_tree(&nodes[after_tf], "(") {
            self.calls.push(CallSite {
                kind: CallKind::Free,
                callee: ident,
                receiver: None,
                span: self.tree.nodes_span(&nodes[start], &nodes[after_tf]),
            });
            self.scan(&nodes[after_tf]);
            return after_tf + 1;
        }

        start + 1
    }

    fn consume_scoped(&mut self, nodes: &[TsNode<'a>], start: usize) -> usize {
        let mut parts = vec![nodes[start].text().into_owned()];
        let mut idx = start + 1;
        while idx + 1 < nodes.len()
            && nodes[idx].text() == "::"
            && nodes[idx + 1].kind() == "identifier"
        {
            parts.push(nodes[idx + 1].text().into_owned());
            idx += 2;
        }

        let after_tf = skip_turbofish(nodes, idx);
        if after_tf < nodes.len() && is_delimited_tree(&nodes[after_tf], "(") {
            if let Some((callee, prefix)) = parts.split_last() {
                if prefix.len() == 1 && matches!(prefix[0].as_str(), "super" | "self") {
                    self.calls.push(CallSite {
                        kind: CallKind::SameModule,
                        callee: callee.clone(),
                        receiver: None,
                        span: self.tree.nodes_span(&nodes[start], &nodes[after_tf]),
                    });
                    self.scan(&nodes[after_tf]);
                    return after_tf + 1;
                }
                let normalized: Vec<&str> = prefix
                    .iter()
                    .map(String::as_str)
                    .filter(|&s| s != "self" && s != "super")
                    .collect();
                let ty_path = normalized.join("::");
                let is_type_ctor = normalized
                    .last()
                    .and_then(|s| s.chars().next())
                    .is_some_and(|c| c.is_ascii_uppercase());
                if is_type_ctor {
                    let ty_ref = syntax::rust_type_ref(ty_path.as_str());
                    if !self.constructed_types.contains(&ty_ref) {
                        self.constructed_types.push(ty_ref);
                    }
                }
                self.calls.push(CallSite {
                    kind: CallKind::Scoped,
                    callee: callee.clone(),
                    receiver: syntax::rust_scoped_receiver(ty_path.clone()),
                    span: self.tree.nodes_span(&nodes[start], &nodes[after_tf]),
                });
                self.scan(&nodes[after_tf]);
                let next = after_tf + 1;
                if is_type_ctor
                    && next + 1 < nodes.len()
                    && nodes[next].text() == "."
                    && nodes[next + 1].kind() == "identifier"
                {
                    return self.consume_dot_chain(nodes, next - 1, &ty_path);
                }
                return next;
            }
        } else if idx < nodes.len() && is_delimited_tree(&nodes[idx], "{") {
            let full_path = parts.join("::");
            self.record_struct_fields(&full_path, &nodes[idx]);
            self.scan(&nodes[idx]);
            return idx + 1;
        }

        idx
    }

    fn consume_dot_chain(
        &mut self,
        nodes: &[TsNode<'a>],
        root_idx: usize,
        root_name: &str,
    ) -> usize {
        let mut idx = root_idx + 1;
        let mut depth = 0usize;

        while idx + 1 < nodes.len()
            && nodes[idx].text() == "."
            && nodes[idx + 1].kind() == "identifier"
        {
            let member = nodes[idx + 1].text().into_owned();
            let after_tf = skip_turbofish(nodes, idx + 2);
            if after_tf < nodes.len() && is_delimited_tree(&nodes[after_tf], "(") {
                let receiver = if depth == 0 {
                    Some(match root_name {
                        "self" => CallReceiver::SelfInstance,
                        "Self" => CallReceiver::SelfType,
                        other => CallReceiver::Named(other.to_string()),
                    })
                } else {
                    None
                };
                self.calls.push(CallSite {
                    kind: CallKind::Method,
                    callee: member,
                    receiver,
                    span: self.tree.nodes_span(&nodes[root_idx], &nodes[after_tf]),
                });
                self.scan(&nodes[after_tf]);
                idx = after_tf + 1;
            } else {
                if depth == 0 {
                    self.field_accesses.push(FieldAccess {
                        receiver_param: root_name.to_string(),
                        field_name: member,
                        is_write: false,
                        span: self.tree.nodes_span(&nodes[root_idx], &nodes[idx + 1]),
                    });
                }
                idx += 2;
            }
            depth += 1;
        }

        idx
    }

    fn record_struct_fields(&mut self, ty_name: &str, brace_tree: &TsNode<'a>) {
        let ty_ref = syntax::rust_type_ref(ty_name);
        if !self.constructed_types.contains(&ty_ref) {
            self.constructed_types.push(ty_ref.clone());
        }
        let inner: Vec<TsNode<'a>> = brace_tree
            .children()
            .filter(|c| !matches!(c.kind().as_ref(), "line_comment" | "block_comment"))
            .collect();
        let mut expect_field = true;
        for (i, tok) in inner.iter().enumerate() {
            let text = tok.text();
            if text == "{" || text == "}" {
                continue;
            }
            if text == "," {
                expect_field = true;
                continue;
            }
            if text == ".." {
                expect_field = false;
                continue;
            }
            if expect_field && tok.kind() == "identifier" {
                let next_text = inner.get(i + 1).map(TsNode::text);
                if matches!(next_text.as_deref(), Some(":" | "," | "}")) {
                    self.field_inits.push(FieldInit {
                        type_name: ty_ref.clone(),
                        field_name: text.into_owned(),
                        span: self.tree.node_span(tok),
                    });
                }
                expect_field = false;
            }
        }
    }
}

fn is_delimited_tree(node: &TsNode<'_>, open: &str) -> bool {
    node.kind() == "token_tree" && node.text().starts_with(open)
}

fn skip_turbofish(nodes: &[TsNode<'_>], idx: usize) -> usize {
    if idx + 1 >= nodes.len() || nodes[idx].text() != "::" || nodes[idx + 1].text() != "<" {
        return idx;
    }
    let mut depth = 0usize;
    let mut cur = idx + 1;
    while cur < nodes.len() {
        let t = nodes[cur].text();
        if t == "<" {
            depth += 1;
        } else if t == ">" {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return cur + 1;
            }
        }
        cur += 1;
    }
    idx
}
