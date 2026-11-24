use super::TsNode;
use super::syntax;
use crate::ast::SyntaxTree;
use crate::ast::item::{
    CallKind, CallReceiver, CallSite, FieldAccess, FieldInit, ReprAssertion, TestHookSite, TypeRef,
};
use std::collections::HashMap;

pub(super) struct MacroTokenData {
    pub(super) field_accesses: Vec<FieldAccess>,
    pub(super) calls: Vec<CallSite>,
    pub(super) constructed_types: Vec<TypeRef>,
    pub(super) field_inits: Vec<FieldInit>,
    pub(super) scoped_ident_refs: Vec<String>,
}

/// Scans unexpanded Rust macro `token_tree` arguments (e.g., `assert_eq!(o.field, helper())`)
/// to extract calls, field accesses, scoped references, and struct literal initializers.
pub(super) struct MacroTokenScanner<'a> {
    tree: &'a SyntaxTree,
    field_accesses: Vec<FieldAccess>,
    calls: Vec<CallSite>,
    constructed_types: Vec<TypeRef>,
    field_inits: Vec<FieldInit>,
    scoped_ident_refs: Vec<String>,
}

impl<'a> MacroTokenScanner<'a> {
    pub(super) fn new(tree: &'a SyntaxTree) -> Self {
        Self {
            tree,
            field_accesses: Vec::new(),
            calls: Vec::new(),
            constructed_types: Vec::new(),
            field_inits: Vec::new(),
            scoped_ident_refs: Vec::new(),
        }
    }

    pub(super) fn scan_token_tree(mut self, tree_node: &TsNode<'a>) -> MacroTokenData {
        self.scan(tree_node);
        MacroTokenData {
            field_accesses: self.field_accesses,
            calls: self.calls,
            constructed_types: self.constructed_types,
            field_inits: self.field_inits,
            scoped_ident_refs: self.scoped_ident_refs,
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
        let after_tf = skip_turbofish(nodes, start + 1);

        if after_tf < nodes.len()
            && is_delimited_tree(&nodes[after_tf], "{")
            && starts_ascii_upper(&ident)
        {
            self.record_struct_fields(&ident, &nodes[after_tf]);
            self.scan(&nodes[after_tf]);
            return after_tf + 1;
        }

        if start + 2 < nodes.len()
            && nodes[start + 1].text() == "."
            && is_dot_member_token(&nodes[start + 2])
        {
            return self.consume_dot_chain(nodes, start, start, &ident, false);
        }

        if after_tf < nodes.len() && is_delimited_tree(&nodes[after_tf], "(") {
            self.calls.push(CallSite {
                kind: CallKind::Free,
                callee: ident,
                receiver: None,
                arg_receivers: Vec::new(),
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
                        arg_receivers: Vec::new(),
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
                let is_type_ctor = normalized.last().is_some_and(|s| starts_ascii_upper(s));
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
                    arg_receivers: Vec::new(),
                    span: self.tree.nodes_span(&nodes[start], &nodes[after_tf]),
                });
                self.scan(&nodes[after_tf]);
                let next = after_tf + 1;
                if is_type_ctor
                    && next + 1 < nodes.len()
                    && nodes[next].text() == "."
                    && is_dot_member_token(&nodes[next + 1])
                {
                    return self.consume_dot_chain(nodes, start, next - 1, &ty_path, true);
                }
                return next;
            }
        } else if after_tf < nodes.len()
            && is_delimited_tree(&nodes[after_tf], "{")
            && parts.last().is_some_and(|s| starts_ascii_upper(s))
        {
            let full_path = parts.join("::");
            self.record_struct_fields(&full_path, &nodes[after_tf]);
            self.scan(&nodes[after_tf]);
            return after_tf + 1;
        } else if let Some(last) = parts.last() {
            self.scoped_ident_refs.push(last.clone());
        }

        idx
    }

    fn consume_dot_chain(
        &mut self,
        nodes: &[TsNode<'a>],
        span_start: usize,
        dot_anchor: usize,
        root_name: &str,
        root_is_type: bool,
    ) -> usize {
        let mut idx = dot_anchor + 1;
        let mut receiver_active = true;

        while idx + 1 < nodes.len()
            && nodes[idx].text() == "."
            && is_dot_member_token(&nodes[idx + 1])
        {
            let member = nodes[idx + 1].text().into_owned();
            let after_tf = skip_turbofish(nodes, idx + 2);
            if after_tf < nodes.len() && is_delimited_tree(&nodes[after_tf], "(") {
                let receiver = receiver_active.then(|| match root_name {
                    "self" => CallReceiver::SelfInstance,
                    "Self" => CallReceiver::SelfType,
                    other => CallReceiver::Named(other.to_string()),
                });
                let preserve_receiver =
                    receiver_active && syntax::is_transparent_chain_method(member.as_str());
                self.calls.push(CallSite {
                    kind: CallKind::Method,
                    callee: member,
                    receiver,
                    arg_receivers: Vec::new(),
                    span: self.tree.nodes_span(&nodes[span_start], &nodes[after_tf]),
                });
                self.scan(&nodes[after_tf]);
                receiver_active = preserve_receiver;
                idx = after_tf + 1;
            } else {
                let (receiver_param, receiver_is_self, receiver_type) = if !receiver_active {
                    (String::new(), false, None)
                } else if root_is_type {
                    (String::new(), false, Some(syntax::rust_type_ref(root_name)))
                } else if root_name == "self" {
                    (String::new(), true, None)
                } else {
                    (root_name.to_string(), false, None)
                };
                self.field_accesses.push(FieldAccess {
                    receiver_param,
                    receiver_is_self,
                    receiver_type,
                    field_name: member,
                    is_read: true,
                    is_write: false,
                    span: self.tree.nodes_span(&nodes[span_start], &nodes[idx + 1]),
                });
                receiver_active = false;
                idx += 2;
            }
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

fn is_dot_member_token(node: &TsNode<'_>) -> bool {
    matches!(node.kind().as_ref(), "identifier" | "integer_literal")
}

pub(super) fn starts_ascii_upper(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_uppercase())
}

pub(super) fn is_delimited_tree(node: &TsNode<'_>, open: &str) -> bool {
    node.kind() == "token_tree" && node.text().starts_with(open)
}

fn is_angle_run(node: &TsNode<'_>, ch: u8) -> bool {
    let t = node.text();
    !t.is_empty() && t.bytes().all(|b| b == ch)
}

pub(super) fn skip_turbofish(nodes: &[TsNode<'_>], idx: usize) -> usize {
    if idx + 1 >= nodes.len() || nodes[idx].text() != "::" || !is_angle_run(&nodes[idx + 1], b'<') {
        return idx;
    }
    let mut depth = 0usize;
    let mut cur = idx + 1;
    while cur < nodes.len() {
        if is_angle_run(&nodes[cur], b'<') {
            depth += nodes[cur].text().len();
        } else if is_angle_run(&nodes[cur], b'>') {
            depth = depth.saturating_sub(nodes[cur].text().len());
            if depth == 0 {
                return cur + 1;
            }
        }
        cur += 1;
    }
    idx
}

pub(super) fn detect_cfg_test_hook(
    tree: &SyntaxTree,
    macro_node: &TsNode<'_>,
) -> Option<TestHookSite> {
    let macro_name = macro_node.field("macro")?.text();
    let trimmed = macro_name.trim();
    if trimmed != "cfg" && !trimmed.ends_with("::cfg") {
        return None;
    }
    let tt = macro_node.children().find(|c| c.kind() == "token_tree")?;
    if !token_tree_contains_ident(&tt, "test") {
        return None;
    }
    Some(TestHookSite {
        label: macro_node.text().trim().to_string(),
        span: tree.node_span(macro_node),
    })
}

fn token_tree_contains_ident(node: &TsNode<'_>, target: &str) -> bool {
    for child in node.children() {
        if child.kind() == "identifier" && child.text() == target {
            return true;
        }
        if child.kind() == "token_tree" && token_tree_contains_ident(&child, target) {
            return true;
        }
    }
    false
}

pub(super) fn detect_debug_repr_assertions(
    tree: &SyntaxTree,
    macro_node: &TsNode<'_>,
) -> Vec<ReprAssertion> {
    let Some(macro_name) = macro_node.field("macro").map(|n| n.text()) else {
        return Vec::new();
    };
    let base = macro_name.trim().rsplit("::").next().unwrap_or("");
    if !matches!(base, "assert_eq" | "debug_assert_eq") {
        return Vec::new();
    }
    let Some(root_tt) = macro_node.children().find(|c| c.kind() == "token_tree") else {
        return Vec::new();
    };
    let (root_tokens, root_args) = split_token_tree_args(&root_tt);
    if root_args.len() < 2 {
        return Vec::new();
    }
    let lhs_fmt = standalone_format_token_tree(&root_tokens[root_args[0].0..root_args[0].1]);
    let rhs_fmt = standalone_format_token_tree(&root_tokens[root_args[1].0..root_args[1].1]);
    let ((Some(fmt_tt), None) | (None, Some(fmt_tt))) = (lhs_fmt, rhs_fmt) else {
        return Vec::new();
    };

    let (fmt_tokens, fmt_args) = split_token_tree_args(&fmt_tt);
    let Some(&(f_start, f_end)) = fmt_args.first() else {
        return Vec::new();
    };
    if f_end != f_start + 1
        || !matches!(fmt_tokens[f_start].kind().as_ref(), "string_literal" | "raw_string_literal")
    {
        return Vec::new();
    }
    let mut named_args: HashMap<String, (usize, usize)> = HashMap::new();
    for &(start, end) in fmt_args.iter().skip(1) {
        if end >= start + 3
            && fmt_tokens[start].kind() == "identifier"
            && fmt_tokens[start + 1].text() == "="
        {
            named_args.insert(fmt_tokens[start].text().into_owned(), (start + 2, end));
        }
    }

    let fmt_text = fmt_tokens[f_start].text();
    let bytes = fmt_text.as_bytes();
    let span = tree.node_span(macro_node);
    let mut out = Vec::new();
    let mut pos = 0usize;
    let mut next_pos = 0usize;

    while pos < bytes.len() {
        if bytes[pos] != b'{' {
            pos += 1;
            continue;
        }
        if bytes.get(pos + 1) == Some(&b'{') {
            pos += 2;
            continue;
        }
        let Some(rel_close) = fmt_text[pos + 1..].find('}') else {
            break;
        };
        let spec = &fmt_text[pos + 1..pos + 1 + rel_close];
        pos = pos + 2 + rel_close;

        let (arg_part, fmt_spec) = spec.split_once(':').unwrap_or((spec, ""));
        let is_debug = fmt_spec.ends_with('?');
        let receiver = if arg_part.is_empty() {
            let idx = next_pos;
            next_pos += 1;
            is_debug.then(|| {
                fmt_args.get(1 + idx).and_then(|&(s, e)| extract_bare_receiver(&fmt_tokens[s..e]))
            }).flatten()
        } else if arg_part.bytes().all(|b| b.is_ascii_digit()) {
            if is_debug
                && let Ok(idx) = arg_part.parse::<usize>()
                && let Some(&(s, e)) = fmt_args.get(1 + idx)
            {
                extract_bare_receiver(&fmt_tokens[s..e])
            } else {
                None
            }
        } else if is_debug && arg_part.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            if let Some(&(s, e)) = named_args.get(arg_part) {
                extract_bare_receiver(&fmt_tokens[s..e])
            } else {
                Some(CallReceiver::Named(arg_part.to_string()))
            }
        } else {
            None
        };
        if let Some(target_receiver) = receiver {
            out.push(ReprAssertion {
                target_receiver: Some(target_receiver),
                span: span.clone(),
            });
        }
    }
    out
}

fn split_token_tree_args<'a>(tt: &TsNode<'a>) -> (Vec<TsNode<'a>>, Vec<(usize, usize)>) {
    let mut tokens: Vec<TsNode<'a>> = tt
        .children()
        .filter(|c| !matches!(c.kind().as_ref(), "line_comment" | "block_comment"))
        .collect();
    if tokens.len() >= 2
        && matches!(
            tokens.first().map(TsNode::text).as_deref(),
            Some("(" | "[" | "{")
        )
        && matches!(
            tokens.last().map(TsNode::text).as_deref(),
            Some(")" | "]" | "}")
        )
    {
        tokens.pop();
        tokens.remove(0);
    }
    let mut ranges = Vec::new();
    let mut start = 0usize;
    for (idx, tok) in tokens.iter().enumerate() {
        if tok.text() == "," {
            if start < idx {
                ranges.push((start, idx));
            }
            start = idx + 1;
        }
    }
    if start < tokens.len() {
        ranges.push((start, tokens.len()));
    }
    (tokens, ranges)
}

fn standalone_format_token_tree<'a>(slice: &[TsNode<'a>]) -> Option<TsNode<'a>> {
    let mut idx = 0usize;
    while idx < slice.len() && matches!(slice[idx].text().as_ref(), "&" | "mut" | "*") {
        idx += 1;
    }
    if slice.len() < idx + 3 {
        return None;
    }
    let last_idx = slice.len() - 1;
    if slice[last_idx].kind() != "token_tree" || slice[last_idx - 1].text() != "!" {
        return None;
    }
    let name_idx = last_idx - 2;
    if slice[name_idx].kind() != "identifier" || slice[name_idx].text() != "format" {
        return None;
    }
    if slice[idx..name_idx]
        .iter()
        .any(|t| t.kind() != "identifier" && t.text() != "::")
    {
        return None;
    }
    Some(slice[last_idx].clone())
}

fn extract_bare_receiver(slice: &[TsNode<'_>]) -> Option<CallReceiver> {
    let mut idx = 0usize;
    while idx < slice.len() && matches!(slice[idx].text().as_ref(), "&" | "mut" | "*") {
        idx += 1;
    }
    if slice.len() != idx + 1 {
        return None;
    }
    match slice[idx].kind().as_ref() {
        "self" => Some(CallReceiver::SelfInstance),
        "identifier" => Some(CallReceiver::Named(slice[idx].text().into_owned())),
        _ => None,
    }
}
