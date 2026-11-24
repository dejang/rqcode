use crate::ast::item::{
    CallKind, CallReceiver, CallSite, ConditionalGuard, FieldAccess, FieldInit, FunctionCouplingFacts,
    FunctionDecl, FunctionRole, LocalBinding, ParamAccess, ParamDecl, TypeRef, Visibility,
};
use crate::ast::similarity::{AstKind, LitKind, NameSpace, NormalizedTree, TreeBuilder};
use crate::ast::{FileRole, LanguageSpec, SyntaxTree};
use crate::diagnostic::Span;
use ast_grep_core::Node;
use ast_grep_core::tree_sitter::StrDoc;
use std::ops::Range;
use std::path::Path;

pub(crate) type CstNode<'a> = Node<'a, StrDoc<LanguageSpec>>;

pub(crate) fn is_comment_kind(kind: &str) -> bool {
    kind == "comment" || kind.ends_with("_comment")
}

#[derive(Clone, Copy)]
pub(crate) struct MemberFieldSpec {
    pub(crate) target_field: &'static str,
    pub(crate) member_field: &'static str,
    pub(crate) ident_kind: &'static str,
    pub(crate) self_kind: Option<&'static str>,
    pub(crate) is_transparent: fn(&str) -> bool,
}

pub(crate) struct FunctionHeader {
    pub(crate) name: String,
    pub(crate) owner_type: Option<String>,
    pub(crate) is_interface_impl: bool,
    pub(crate) interface_name: Option<TypeRef>,
    pub(crate) self_receiver: Option<ParamAccess>,
    pub(crate) visibility: Visibility,
    pub(crate) explicit_visibility: bool,
    pub(crate) is_default_export: bool,
    pub(crate) role: FunctionRole,
    pub(crate) params: Vec<ParamDecl>,
    pub(crate) return_type: Option<TypeRef>,
}

#[derive(Default)]
pub(crate) struct ExtractedBody {
    pub(crate) body_tree: NormalizedTree,
    pub(crate) field_accesses: Vec<FieldAccess>,
    pub(crate) calls: Vec<CallSite>,
    pub(crate) constructed_types: Vec<TypeRef>,
    pub(crate) local_bindings: Vec<LocalBinding>,
    pub(crate) field_inits: Vec<FieldInit>,
    pub(crate) referenced_idents: Vec<String>,
    pub(crate) scoped_ident_refs: Vec<String>,
    pub(crate) coupling: FunctionCouplingFacts,
}

impl ExtractedBody {
    pub(crate) fn into_function_decl(self, header: FunctionHeader, span: Span) -> FunctionDecl {
        FunctionDecl {
            name: header.name,
            owner_type: header.owner_type,
            is_interface_impl: header.is_interface_impl,
            interface_name: header.interface_name,
            self_receiver: header.self_receiver,
            visibility: header.visibility,
            explicit_visibility: header.explicit_visibility,
            is_default_export: header.is_default_export,
            role: header.role,
            conditional_guard: ConditionalGuard::None,
            params: header.params,
            return_type: header.return_type,
            body_tree: self.body_tree,
            field_accesses: self.field_accesses,
            calls: self.calls,
            constructed_types: self.constructed_types,
            local_bindings: self.local_bindings,
            field_inits: self.field_inits,
            referenced_idents: self.referenced_idents,
            scoped_ident_refs: self.scoped_ident_refs,
            coupling: self.coupling,
            span,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AccessMode {
    Read,
    Write,
    ReadWrite,
}

impl AccessMode {
    pub(crate) const fn is_read(self) -> bool {
        matches!(self, Self::Read | Self::ReadWrite)
    }

    pub(crate) const fn is_write(self) -> bool {
        matches!(self, Self::Write | Self::ReadWrite)
    }
}

#[derive(Default)]
pub(crate) struct BodyAccumulator {
    out: ExtractedBody,
}

impl BodyAccumulator {
    pub(crate) fn set_body_tree(&mut self, tree: NormalizedTree) {
        self.out.body_tree = tree;
    }
    pub(crate) const fn coupling_mut(&mut self) -> &mut FunctionCouplingFacts {
        &mut self.out.coupling
    }

    pub(crate) fn push_field_access(&mut self, access: FieldAccess) {
        self.out.field_accesses.push(access);
    }

    /// Record a direct field access when `node.field(spec.target_field)` unwraps through
    /// transparent wrappers to a plain identifier (`spec.ident_kind`) or `self`/`this`
    /// (`spec.self_kind`), and record an unresolved access for chained targets (e.g., `a.b.c`)
    /// so production field reads remain observable. Returns `Some((raw_target, recorded))` where
    /// `recorded` is `true` for a direct receiver access.
    pub(crate) fn record_ident_field_access<'a>(
        &mut self,
        tree: &SyntaxTree,
        node: &CstNode<'a>,
        spec: MemberFieldSpec,
        mode: AccessMode,
        fallback_type: Option<TypeRef>,
    ) -> Option<(CstNode<'a>, bool)> {
        let raw_target = node.field(spec.target_field)?;
        let mut target = raw_target.clone();
        while (spec.is_transparent)(target.kind().as_ref()) {
            let Some(inner) = target
                .children()
                .find(|c| c.is_named() && !is_comment_kind(c.kind().as_ref()))
            else {
                break;
            };
            target = inner;
        }
        let target_kind = target.kind();
        let is_self = spec.self_kind.is_some_and(|k| target_kind == k);
        let direct = is_self || target_kind == spec.ident_kind;
        if let Some(field) = node.field(spec.member_field) {
            let receiver_param = if direct && !is_self {
                target.text().into_owned()
            } else {
                String::new()
            };
            self.push_field_access(FieldAccess {
                receiver_param,
                receiver_is_self: is_self,
                receiver_type: if direct { None } else { fallback_type },
                field_name: field.text().into_owned(),
                is_read: mode.is_read(),
                is_write: mode.is_write(),
                span: tree.node_span(node),
            });
        }
        Some((raw_target, direct))
    }

    pub(crate) fn push_call(&mut self, call: CallSite) {
        self.out.calls.push(call);
    }

    pub(crate) fn push_constructed_type(&mut self, ty_ref: TypeRef) {
        self.out.constructed_types.push(ty_ref);
    }

    pub(crate) fn push_local_binding(&mut self, binding: LocalBinding) {
        self.out.local_bindings.push(binding);
    }

    pub(crate) fn push_field_init(&mut self, init: FieldInit) {
        self.out.field_inits.push(init);
    }

    pub(crate) fn push_referenced_ident(&mut self, ident: String) {
        self.out.referenced_idents.push(ident);
    }

    pub(crate) fn push_scoped_ident_ref(&mut self, ident: String) {
        self.out.scoped_ident_refs.push(ident);
    }

    pub(crate) fn record_scoped_type_call(
        &mut self,
        receiver: CallReceiver,
        constructed: Option<TypeRef>,
        callee: String,
        arg_receivers: Vec<Option<CallReceiver>>,
        span: Span,
    ) {
        if let Some(ty_ref) = constructed {
            self.push_constructed_type(ty_ref);
        }
        self.push_scoped_ident_ref(callee.clone());
        self.push_call(CallSite {
            kind: CallKind::Scoped,
            callee,
            receiver: Some(receiver),
            arg_receivers,
            span,
        });
    }

    pub(crate) fn extend_macro_data(
        &mut self,
        field_accesses: Vec<FieldAccess>,
        calls: Vec<CallSite>,
        constructed_types: Vec<TypeRef>,
        field_inits: Vec<FieldInit>,
        scoped_ident_refs: Vec<String>,
    ) {
        self.out.field_accesses.extend(field_accesses);
        self.out.calls.extend(calls);
        self.out.constructed_types.extend(constructed_types);
        self.out.field_inits.extend(field_inits);
        self.out.scoped_ident_refs.extend(scoped_ident_refs);
    }

    pub(crate) fn finish(mut self) -> ExtractedBody {
        self.out.constructed_types.sort_unstable();
        self.out.constructed_types.dedup();
        self.out.referenced_idents.sort_unstable();
        self.out.referenced_idents.dedup();
        self.out.scoped_ident_refs.sort_unstable();
        self.out.scoped_ident_refs.dedup();
        self.out
    }
}

/// Language-specific semantic body walker paired with a [`CstNormalizeDialect`].
pub(crate) trait BodyMetadataWalker<'a> {
    fn walk_body(&mut self, body: &CstNode<'a>);
    fn into_accumulator(self) -> BodyAccumulator;
}

/// Extract both semantic body facts and Stage 1 [`NormalizedTree`] with a dialect.
pub(crate) fn extract_body_with_dialect<'a, D: CstNormalizeDialect, W: BodyMetadataWalker<'a>>(
    mut walker: W,
    mut dialect: D,
    body: Option<CstNode<'a>>,
) -> ExtractedBody {
    let body_tree = if let Some(body) = body {
        walker.walk_body(&body);
        CstNormalizer::default().normalize(&mut dialect, &body)
    } else {
        NormalizedTree::default()
    };
    let mut acc = walker.into_accumulator();
    acc.set_body_tree(body_tree);
    acc.finish()
}

/// Per-language CST normalization rules plugged into [`CstNormalizer`].
pub(crate) trait CstNormalizeDialect: Sized {
    fn visit_compound(&mut self, norm: &mut CstNormalizer, node: &CstNode<'_>) -> bool;
    fn visit_special(&mut self, norm: &mut CstNormalizer, node: &CstNode<'_>) -> bool;
    fn visit_callee(&mut self, norm: &mut CstNormalizer, node: &CstNode<'_>);
    fn visit_leaf(&mut self, norm: &mut CstNormalizer, node: &CstNode<'_>);

    fn walk_call_args(&mut self, norm: &mut CstNormalizer, args: &CstNode<'_>) {
        norm.walk_named_children(self, args);
    }
}

/// Shared tree-sitter CST-to-[`NormalizedTree`] driver used across language frontends.
pub(crate) struct CstNormalizer {
    tb: TreeBuilder,
    depth: usize,
    max_depth: usize,
}

impl Default for CstNormalizer {
    fn default() -> Self {
        Self::with_max_depth(512)
    }
}

impl CstNormalizer {
    pub(crate) fn with_max_depth(max_depth: usize) -> Self {
        Self {
            tb: TreeBuilder::new(),
            depth: 0,
            max_depth,
        }
    }

    pub(crate) fn normalize<D: CstNormalizeDialect>(
        mut self,
        dialect: &mut D,
        body: &CstNode<'_>,
    ) -> NormalizedTree {
        self.walk(dialect, body);
        self.tb.finish()
    }

    pub(crate) fn tb(&mut self) -> &mut TreeBuilder {
        &mut self.tb
    }

    pub(crate) fn with_depth<F: FnOnce(&mut Self)>(&mut self, f: F) {
        if self.depth >= self.max_depth {
            return;
        }
        self.depth += 1;
        f(self);
        self.depth -= 1;
    }

    pub(crate) fn walk<D: CstNormalizeDialect>(&mut self, dialect: &mut D, node: &CstNode<'_>) {
        self.with_depth(|this| {
            if !dialect.visit_compound(this, node) && !dialect.visit_special(this, node) {
                if node.is_leaf() {
                    dialect.visit_leaf(this, node);
                } else {
                    for child in node.children() {
                        this.walk(dialect, &child);
                    }
                }
            }
        });
    }

    pub(crate) fn wrap_children<D: CstNormalizeDialect>(
        &mut self,
        dialect: &mut D,
        kind: AstKind,
        node: &CstNode<'_>,
    ) -> bool {
        let mark = self.tb.enter(kind);
        for child in node.children() {
            self.walk(dialect, &child);
        }
        self.tb.exit(mark);
        true
    }

    pub(crate) fn walk_named_children<D: CstNormalizeDialect>(
        &mut self,
        dialect: &mut D,
        node: &CstNode<'_>,
    ) {
        for child in node.children() {
            if child.is_named() {
                self.walk(dialect, &child);
            }
        }
    }

    pub(crate) fn walk_named_except<D: CstNormalizeDialect>(
        &mut self,
        dialect: &mut D,
        node: &CstNode<'_>,
        skip_range: Option<&Range<usize>>,
    ) {
        for child in node.children() {
            if child.is_named() && skip_range != Some(&child.range()) {
                self.walk(dialect, &child);
            }
        }
    }

    pub(crate) fn walk_interpolated_string<D: CstNormalizeDialect>(
        &mut self,
        dialect: &mut D,
        node: &CstNode<'_>,
        interp_kind: &str,
    ) {
        let has_interp = node.children().any(|c| c.kind() == interp_kind);
        if !has_interp {
            self.tb.literal(LitKind::String);
            return;
        }
        let mark = self.tb.enter(AstKind::Other);
        self.tb.literal(LitKind::String);
        for child in node.children() {
            if child.kind() == interp_kind {
                self.walk_named_children(dialect, &child);
            }
        }
        self.tb.exit(mark);
    }

    pub(crate) fn walk_tagged_named_children<D: CstNormalizeDialect>(
        &mut self,
        dialect: &mut D,
        kind: AstKind,
        tag: &str,
        node: &CstNode<'_>,
    ) {
        let mark = self.tb.enter(kind);
        self.tb.exact(tag);
        self.walk_named_children(dialect, node);
        self.tb.exit(mark);
    }

    pub(crate) fn walk_unary_fields<D: CstNormalizeDialect>(
        &mut self,
        dialect: &mut D,
        node: &CstNode<'_>,
        op_field: &str,
        operand_field: &str,
    ) {
        let mark = self.tb.enter(AstKind::Unary);
        let op_range = node
            .field(op_field)
            .or_else(|| node.children().find(|c| !c.is_named()))
            .map(|op| {
                self.tb.exact(op.text().trim());
                op.range()
            });
        if let Some(operand) = node.field(operand_field) {
            self.walk(dialect, &operand);
        } else {
            self.walk_named_except(dialect, node, op_range.as_ref());
        }
        self.tb.exit(mark);
    }

    pub(crate) fn walk_assignment<D: CstNormalizeDialect>(
        &mut self,
        dialect: &mut D,
        node: &CstNode<'_>,
        lhs_field: &str,
        rhs_field: &str,
    ) {
        let mark = self.tb.enter(AstKind::Assign);
        if let Some(op) = node.field("operator") {
            self.tb.exact(op.text().trim());
        } else {
            self.tb.exact("=");
        }
        if let Some(lhs) = node.field(lhs_field) {
            self.walk(dialect, &lhs);
        }
        if let Some(rhs) = node.field(rhs_field) {
            self.walk(dialect, &rhs);
        }
        self.tb.exit(mark);
    }

    pub(crate) fn walk_binary<D: CstNormalizeDialect>(
        &mut self,
        dialect: &mut D,
        node: &CstNode<'_>,
        lhs_field: &str,
        rhs_field: &str,
    ) {
        let mark = self.tb.enter(AstKind::Binary);
        if let Some(op) = node
            .field("operator")
            .or_else(|| node.field("op"))
            .or_else(|| node.children().find(|c| !c.is_named()))
        {
            self.tb.exact(op.text().trim());
        }
        let lhs = node.field(lhs_field);
        let rhs = node.field(rhs_field);
        if lhs.is_none() && rhs.is_none() {
            self.walk_named_children(dialect, node);
        } else {
            if let Some(l) = lhs {
                self.walk(dialect, &l);
            }
            if let Some(r) = rhs {
                self.walk(dialect, &r);
            }
        }
        self.tb.exit(mark);
    }

    pub(crate) fn walk_member_access<D: CstNormalizeDialect>(
        &mut self,
        dialect: &mut D,
        node: &CstNode<'_>,
        obj_field: &str,
        prop_field: &str,
    ) {
        let mark = self.tb.enter(AstKind::Field);
        if let Some(obj) = node.field(obj_field) {
            self.walk(dialect, &obj);
        }
        if let Some(prop) = node.field(prop_field) {
            self.tb.symbol(NameSpace::Member, prop.text().trim());
        }
        self.tb.exit(mark);
    }

    pub(crate) fn walk_binding<D: CstNormalizeDialect>(
        &mut self,
        dialect: &mut D,
        node: &CstNode<'_>,
        name_field: &str,
        val_field: &str,
    ) {
        let mark = self.tb.enter(AstKind::Bind);
        if let Some(name) = node.field(name_field) {
            self.walk(dialect, &name);
        }
        if let Some(val) = node.field(val_field) {
            self.walk(dialect, &val);
        }
        self.tb.exit(mark);
    }

    pub(crate) fn walk_call<D: CstNormalizeDialect>(
        &mut self,
        dialect: &mut D,
        node: &CstNode<'_>,
        callee_field: &str,
        args_field: &str,
    ) {
        let mark = self.tb.enter(AstKind::Call);
        if let Some(func) = node.field(callee_field) {
            dialect.visit_callee(self, &func);
        }
        if let Some(args) = node.field(args_field) {
            dialect.walk_call_args(self, &args);
        }
        self.tb.exit(mark);
    }

    pub(crate) fn walk_labeled_arg<D: CstNormalizeDialect>(
        &mut self,
        dialect: &mut D,
        label: Option<&str>,
        value: Option<&CstNode<'_>>,
    ) {
        let arg = self.tb.enter(AstKind::Arg);
        if let Some(lbl) = label {
            self.tb.symbol(NameSpace::Label, lbl);
        }
        if let Some(val) = value {
            self.walk(dialect, val);
        }
        self.tb.exit(arg);
    }

    pub(crate) fn emit_shorthand_arg(&mut self, name: &str) {
        let arg = self.tb.enter(AstKind::Arg);
        self.tb.symbol(NameSpace::Label, name);
        self.tb.symbol(NameSpace::Plain, name);
        self.tb.exit(arg);
    }

    pub(crate) fn emit_leaf_fallback(&mut self, node: &CstNode<'_>) {
        let text = node.text();
        let trimmed = text.trim();
        if !trimmed.is_empty() {
            self.tb.exact(trimmed);
        }
    }
}

pub(crate) fn collect_leaf_idents(node: &CstNode<'_>, ident_kind: &str, out: &mut Vec<String>) {
    if node.is_leaf() {
        if node.kind().as_ref() == ident_kind {
            out.push(node.text().into_owned());
        }
        return;
    }
    for child in node.children() {
        collect_leaf_idents(&child, ident_kind, out);
    }
}

pub(crate) fn classify_file_role(
    relative_path: &Path,
    integration_dirs: &[&str],
    unit_test_suffixes: &[&str],
    lib_roots: &[&str],
    bin_entries: &[&str],
) -> FileRole {
    let first = relative_path.components().next().and_then(|c| c.as_os_str().to_str());
    if first.is_some_and(|f| integration_dirs.contains(&f)) {
        return FileRole::IntegrationTest;
    }
    let Some(name) = relative_path.file_name().and_then(|n| n.to_str()) else {
        return FileRole::Module;
    };
    if unit_test_suffixes.iter().any(|s| name.ends_with(s)) {
        FileRole::UnitTest
    } else if lib_roots.contains(&name) {
        FileRole::LibraryRoot
    } else if bin_entries.contains(&name) {
        FileRole::BinaryEntrypoint
    } else {
        FileRole::Module
    }
}
