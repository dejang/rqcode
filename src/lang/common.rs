use crate::ast::item::{CallSite, FieldAccess, FieldInit, LocalBinding, TypeRef};
use crate::ast::vector::{NormToken, NormalizedAst};
use crate::ast::{FileRole, LanguageSpec};
use ast_grep_core::Node;
use ast_grep_core::tree_sitter::StrDoc;
use std::path::Path;

pub(crate) type CstNode<'a> = Node<'a, StrDoc<LanguageSpec>>;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum LeafClass {
    Ident,
    Lit,
    Skip,
    Other,
}

#[derive(Default)]
pub(crate) struct ExtractedBody {
    pub(crate) body_tokens: NormalizedAst,
    pub(crate) field_accesses: Vec<FieldAccess>,
    pub(crate) calls: Vec<CallSite>,
    pub(crate) constructed_types: Vec<TypeRef>,
    pub(crate) local_bindings: Vec<LocalBinding>,
    pub(crate) field_inits: Vec<FieldInit>,
    pub(crate) referenced_idents: Vec<String>,
    pub(crate) scoped_ident_refs: Vec<String>,
}

#[derive(Default)]
pub(crate) struct BodyAccumulator {
    tokens: Vec<NormToken>,
    out: ExtractedBody,
}

impl BodyAccumulator {
    pub(crate) fn push_token(&mut self, token: NormToken) {
        self.tokens.push(token);
    }

    pub(crate) fn push_field_access(&mut self, access: FieldAccess) {
        self.out.field_accesses.push(access);
    }

    pub(crate) fn push_call(&mut self, call: CallSite) {
        self.out.calls.push(call);
    }

    pub(crate) fn push_constructed_type(&mut self, ty_ref: TypeRef) {
        if !self.out.constructed_types.contains(&ty_ref) {
            self.out.constructed_types.push(ty_ref);
        }
    }

    pub(crate) fn push_local_binding(&mut self, binding: LocalBinding) {
        self.out.local_bindings.push(binding);
    }

    pub(crate) fn push_field_init(&mut self, init: FieldInit) {
        self.out.field_inits.push(init);
    }

    pub(crate) fn push_scoped_ident_ref(&mut self, ident: String) {
        self.out.scoped_ident_refs.push(ident);
    }

    pub(crate) fn extend_macro_data(
        &mut self,
        field_accesses: Vec<FieldAccess>,
        calls: Vec<CallSite>,
        constructed_types: Vec<TypeRef>,
        field_inits: Vec<FieldInit>,
    ) {
        self.out.field_accesses.extend(field_accesses);
        self.out.calls.extend(calls);
        self.out.constructed_types.extend(constructed_types);
        self.out.field_inits.extend(field_inits);
    }

    pub(crate) fn collect_scoped_idents(&mut self, node: &CstNode<'_>) {
        collect_leaf_idents(node, &mut self.out.scoped_ident_refs);
    }

    pub(crate) fn record_leaf(&mut self, node: &CstNode<'_>, class: LeafClass) {
        if node.kind().as_ref() == "identifier" {
            self.out.referenced_idents.push(node.text().into_owned());
        }
        match class {
            LeafClass::Ident => self.tokens.push(NormToken::Ident),
            LeafClass::Lit => self.tokens.push(NormToken::Lit),
            LeafClass::Skip => {}
            LeafClass::Other => {
                let text = node.text();
                let trimmed = text.trim();
                if !trimmed.is_empty() {
                    self.tokens.push(NormToken::Other(trimmed.to_string()));
                }
            }
        }
    }

    pub(crate) fn finish(mut self) -> ExtractedBody {
        self.out.constructed_types.sort_unstable();
        self.out.constructed_types.dedup();
        self.out.referenced_idents.sort_unstable();
        self.out.referenced_idents.dedup();
        self.out.scoped_ident_refs.sort_unstable();
        self.out.scoped_ident_refs.dedup();
        self.out.body_tokens = NormalizedAst::from_norm_tokens(&self.tokens);
        self.out
    }
}

pub(crate) fn collect_leaf_idents(node: &CstNode<'_>, out: &mut Vec<String>) {
    if node.is_leaf() {
        if node.kind().as_ref() == "identifier" {
            out.push(node.text().into_owned());
        }
        return;
    }
    for child in node.children() {
        collect_leaf_idents(&child, out);
    }
}

pub(crate) fn classify_file_role(
    relative_path: &Path,
    integration_dirs: &[&str],
    unit_test_suffixes: &[&str],
    lib_roots: &[&str],
    bin_entries: &[&str],
) -> FileRole {
    if let Some(first) = relative_path
        .components()
        .next()
        .and_then(|c| c.as_os_str().to_str())
        && integration_dirs.contains(&first)
    {
        return FileRole::IntegrationTest;
    }
    let Some(file_name) = relative_path.file_name().and_then(|n| n.to_str()) else {
        return FileRole::Module;
    };
    for &suffix in unit_test_suffixes {
        if file_name.ends_with(suffix) {
            return FileRole::UnitTest;
        }
    }
    if lib_roots.contains(&file_name) {
        return FileRole::LibraryRoot;
    }
    if bin_entries.contains(&file_name) {
        return FileRole::BinaryEntrypoint;
    }
    FileRole::Module
}
