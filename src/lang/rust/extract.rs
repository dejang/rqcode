use super::TsNode;
use super::body::{BodyWalker, ExtractedBody};
use super::syntax;
use crate::ast::item::{
    ExportDecl, FieldDecl, FunctionDecl, FunctionRole, ImportDecl, ImportKind, ParamAccess,
    ParamDecl, TypeDecl, TypeKind, TypeRef, Visibility,
};
use crate::ast::{FileAst, FileRole, ModulePath, SyntaxTree};
use crate::diagnostic::Span;

struct UseContext<'a> {
    in_test_module: bool,
    is_reexport: bool,
    span: &'a Span,
}

pub(super) struct RustExtractor<'a> {
    tree: &'a SyntaxTree,
    module_path: ModulePath,
    imports: Vec<ImportDecl>,
    exports: Vec<ExportDecl>,
    types: Vec<TypeDecl>,
    functions: Vec<FunctionDecl>,
    top_level_idents: Vec<String>,
}

impl<'a> RustExtractor<'a> {
    pub(super) fn new(tree: &'a SyntaxTree, module_path: ModulePath) -> Self {
        Self {
            tree,
            module_path,
            imports: Vec::new(),
            exports: Vec::new(),
            types: Vec::new(),
            functions: Vec::new(),
            top_level_idents: Vec::new(),
        }
    }

    pub(super) fn extract_file(mut self, role: FileRole) -> FileAst {
        self.walk_item_list(&self.tree.root(), false);
        self.top_level_idents.sort_unstable();
        self.top_level_idents.dedup();

        FileAst {
            path: self.tree.path().to_path_buf(),
            relative_path: self.tree.relative_path().to_path_buf(),
            module_path: self.module_path,
            language: self.tree.language_id(),
            member_separator: self.tree.language().member_separator(),
            role,
            imports: self.imports,
            exports: self.exports,
            types: self.types,
            functions: self.functions,
            top_level_idents: self.top_level_idents,
            suppressions: self.tree.extract_suppressions(),
        }
    }

    fn walk_item_list(&mut self, container: &TsNode<'a>, in_test_module: bool) {
        let mut pending_attrs: Vec<String> = Vec::new();
        let mut pending_has_cfg_test = false;
        let mut compact_attr = String::new();

        for child in container.children() {
            let kind = child.kind();
            match kind.as_ref() {
                "attribute_item" => {
                    let text = child.text();
                    compact_attr.clear();
                    compact_attr.extend(text.chars().filter(|c| !c.is_whitespace()));
                    if compact_attr.contains("cfg(test)") {
                        pending_has_cfg_test = true;
                    }
                    pending_attrs.push(text.into_owned());
                }
                "line_comment" | "block_comment" => {}
                "use_declaration" => {
                    self.extract_use(&child, in_test_module);
                    pending_attrs.clear();
                    pending_has_cfg_test = false;
                }
                "struct_item" => {
                    if let Some(ty) = self.extract_struct(&child, &pending_attrs, in_test_module) {
                        self.types.push(ty);
                    }
                    pending_attrs.clear();
                    pending_has_cfg_test = false;
                }
                "enum_item" => {
                    if let Some(ty) = self.extract_enum(&child, &pending_attrs, in_test_module) {
                        self.types.push(ty);
                    }
                    pending_attrs.clear();
                    pending_has_cfg_test = false;
                }
                "impl_item" => {
                    self.extract_impl(&child, in_test_module);
                    pending_attrs.clear();
                    pending_has_cfg_test = false;
                }
                "function_item" => {
                    if let Some(func) =
                        self.extract_function(&child, &pending_attrs, None, false, in_test_module)
                    {
                        self.functions.push(func);
                    }
                    pending_attrs.clear();
                    pending_has_cfg_test = false;
                }
                "const_item" | "static_item" => {
                    if !in_test_module && let Some(val) = child.field("value") {
                        syntax::collect_leaf_idents(&val, &mut self.top_level_idents);
                    }
                    pending_attrs.clear();
                    pending_has_cfg_test = false;
                }
                "mod_item" => {
                    let mod_name = child
                        .field("name")
                        .map(|n| n.text().into_owned())
                        .unwrap_or_default();
                    let child_is_test =
                        in_test_module || mod_name == "tests" || pending_has_cfg_test;
                    if let Some(body) = child.field("body") {
                        self.walk_item_list(&body, child_is_test);
                    }
                    pending_attrs.clear();
                    pending_has_cfg_test = false;
                }
                _ => {
                    pending_attrs.clear();
                    pending_has_cfg_test = false;
                }
            }
        }
    }

    fn extract_use(&mut self, node: &TsNode<'a>, in_test_module: bool) {
        if let Some(arg) = node.field("argument") {
            let span = self.tree.node_span(node);
            let ctx = UseContext {
                in_test_module,
                is_reexport: !in_test_module && extract_visibility(node).is_externally_visible(),
                span: &span,
            };
            let mut prefix = Vec::new();
            self.collect_use_tree(&arg, &mut prefix, &ctx);
        }
    }

    fn resolve_import_module(&self, raw_segments: &[String]) -> Option<ModulePath> {
        let first = raw_segments.first()?.as_str();
        let mut resolved = Vec::new();
        match first {
            "crate" => {
                resolved.push(String::from("crate"));
                for seg in &raw_segments[1..] {
                    apply_rust_module_segment(&mut resolved, seg)?;
                }
            }
            "self" | "super" => {
                resolved.extend(self.module_path.segments().iter().cloned());
                for seg in raw_segments {
                    apply_rust_module_segment(&mut resolved, seg)?;
                }
            }
            _ => return None,
        }
        Some(ModulePath::new(resolved))
    }

    fn record_import(
        &mut self,
        raw_segments: Vec<String>,
        kind: ImportKind,
        symbol: String,
        alias: Option<String>,
        ctx: &UseContext<'_>,
    ) {
        let resolved_module = self.resolve_import_module(&raw_segments);
        let module_segments = match &resolved_module {
            Some(resolved) => resolved.segments().to_vec(),
            None => raw_segments,
        };
        let normalized_symbol = if symbol == "self" {
            String::new()
        } else {
            symbol
        };

        if ctx.is_reexport
            && let Some(src_mod) = resolved_module.clone()
        {
            if kind == ImportKind::Wildcard {
                self.exports.push(ExportDecl {
                    source_module: Some(src_mod),
                    symbol: String::new(),
                    exported_name: String::new(),
                    is_default: false,
                    is_wildcard: true,
                });
            } else if !normalized_symbol.is_empty() {
                let exported_name = alias.clone().unwrap_or_else(|| normalized_symbol.clone());
                self.exports.push(ExportDecl {
                    source_module: Some(src_mod),
                    symbol: normalized_symbol.clone(),
                    exported_name,
                    is_default: false,
                    is_wildcard: false,
                });
            }
        }

        self.imports.push(ImportDecl {
            resolved_module,
            module_segments,
            kind,
            symbol: normalized_symbol,
            alias,
            in_test_module: ctx.in_test_module,
            span: ctx.span.clone(),
        });
    }

    fn collect_use_tree(
        &mut self,
        node: &TsNode<'a>,
        prefix: &mut Vec<String>,
        ctx: &UseContext<'_>,
    ) {
        match node.kind().as_ref() {
            "identifier" => {
                let symbol = node.text().into_owned();
                if symbol != "self" {
                    self.record_import(prefix.clone(), ImportKind::Named, symbol, None, ctx);
                }
            }
            "scoped_identifier" => {
                let text = node.text();
                let mut segs = prefix.clone();
                segs.extend(text.split("::").filter(|s| !s.is_empty()).map(String::from));
                if let Some(symbol) = segs.pop() {
                    self.record_import(segs, ImportKind::Named, symbol, None, ctx);
                }
            }
            "use_as_clause" => {
                let alias = node.field("alias").map(|n| n.text().into_owned());
                if let Some(path) = node.field("path") {
                    let text = path.text();
                    let mut segs = prefix.clone();
                    segs.extend(text.split("::").filter(|s| !s.is_empty()).map(String::from));
                    if let Some(symbol) = segs.pop() {
                        self.record_import(segs, ImportKind::Named, symbol, alias, ctx);
                    }
                }
            }
            "use_wildcard" => {
                let text = node.text();
                let path_part = text.trim().trim_end_matches('*').trim_end_matches("::");
                let mut segs = prefix.clone();
                segs.extend(
                    path_part
                        .split("::")
                        .filter(|s| !s.is_empty())
                        .map(String::from),
                );
                if !(ctx.in_test_module && segs.len() == 1 && segs[0] == "super") {
                    self.record_import(segs, ImportKind::Wildcard, "*".to_string(), None, ctx);
                }
            }
            "scoped_use_list" => {
                let saved_len = prefix.len();
                if let Some(path) = node.field("path") {
                    let text = path.text();
                    prefix.extend(text.split("::").filter(|s| !s.is_empty()).map(String::from));
                }
                if let Some(list) = node.field("list") {
                    for child in list.children() {
                        if child.is_named() {
                            self.collect_use_tree(&child, prefix, ctx);
                        }
                    }
                }
                prefix.truncate(saved_len);
            }
            "use_list" => {
                for child in node.children() {
                    if child.is_named() {
                        self.collect_use_tree(&child, prefix, ctx);
                    }
                }
            }
            _ => {}
        }
    }

    fn extract_struct(
        &self,
        node: &TsNode<'a>,
        attrs: &[String],
        in_test_module: bool,
    ) -> Option<TypeDecl> {
        let name = node.field("name")?.text().into_owned();
        let visibility = extract_visibility(node);
        let annotations = extract_derives(attrs);
        let mut fields = Vec::new();

        if let Some(body) = node.field("body")
            && body.kind() == "field_declaration_list"
        {
            for child in body.children() {
                if child.kind() == "field_declaration" {
                    let Some(field_name) = child.field("name").map(|n| n.text().into_owned())
                    else {
                        continue;
                    };
                    let (field_type, field_access) = child
                        .field("type")
                        .map_or((None, ParamAccess::Unknown), |t| {
                            syntax::extract_base_type(&t)
                        });
                    fields.push(FieldDecl {
                        name: field_name,
                        type_name: field_type,
                        visibility: extract_visibility(&child),
                        is_mut_ref: field_access.is_mutable(),
                        span: self.tree.node_span(&child),
                    });
                }
            }
        }

        Some(TypeDecl {
            name,
            kind: TypeKind::Struct,
            visibility,
            in_test_module,
            is_default_export: false,
            fields,
            annotations,
            span: self.tree.node_span(node),
        })
    }

    fn extract_enum(
        &self,
        node: &TsNode<'a>,
        attrs: &[String],
        in_test_module: bool,
    ) -> Option<TypeDecl> {
        let name = node.field("name")?.text().into_owned();
        Some(TypeDecl {
            name,
            kind: TypeKind::Enum,
            visibility: extract_visibility(node),
            in_test_module,
            is_default_export: false,
            fields: Vec::new(),
            annotations: extract_derives(attrs),
            span: self.tree.node_span(node),
        })
    }

    fn extract_impl(&mut self, node: &TsNode<'a>, in_test_module: bool) {
        let is_interface_impl = node.field("trait").is_some();
        let owner_type = node
            .field("type")
            .and_then(|t| match syntax::extract_base_type(&t).0 {
                Some(TypeRef::Named(name)) => Some(name),
                _ => None,
            });

        let Some(body) = node.field("body") else {
            return;
        };

        let mut pending_attrs = Vec::new();
        for child in body.children() {
            match child.kind().as_ref() {
                "attribute_item" => pending_attrs.push(child.text().into_owned()),
                "line_comment" | "block_comment" => {}
                "function_item" => {
                    if let Some(func) = self.extract_function(
                        &child,
                        &pending_attrs,
                        owner_type.clone(),
                        is_interface_impl,
                        in_test_module,
                    ) {
                        self.functions.push(func);
                    }
                    pending_attrs.clear();
                }
                _ => pending_attrs.clear(),
            }
        }
    }

    fn extract_function(
        &self,
        node: &TsNode<'a>,
        attrs: &[String],
        owner_type: Option<String>,
        is_interface_impl: bool,
        in_test_module: bool,
    ) -> Option<FunctionDecl> {
        let name = node.field("name")?.text().into_owned();
        let visibility = extract_visibility(node);

        let mut is_test = false;
        let mut compact = String::new();
        for attr in attrs {
            compact.clear();
            compact.extend(attr.chars().filter(|c| !c.is_whitespace()));
            if compact == "#[test]"
                || compact.ends_with("::test]")
                || compact.contains("::test(")
                || compact.starts_with("#[rstest")
            {
                is_test = true;
                break;
            }
        }

        let role = if is_test {
            FunctionRole::Test
        } else if in_test_module {
            FunctionRole::TestHelper
        } else if owner_type.is_none() && name == "main" {
            FunctionRole::Entrypoint
        } else {
            FunctionRole::Production
        };

        let mut self_receiver = None;
        let mut params = Vec::new();
        if let Some(param_list) = node.field("parameters") {
            for child in param_list.children() {
                if child.kind() == "self_parameter" {
                    let mut is_ref = false;
                    let mut is_mut = false;
                    for part in child.children() {
                        match part.kind().as_ref() {
                            "&" => is_ref = true,
                            "mutable_specifier" => is_mut = true,
                            _ => {}
                        }
                    }
                    self_receiver = Some(match (is_ref, is_mut) {
                        (true, true) => ParamAccess::MutableRef,
                        (true, false) => ParamAccess::SharedRef,
                        (false, _) => ParamAccess::Owned,
                    });
                } else if child.kind() == "parameter" {
                    let param_name = child
                        .field("pattern")
                        .and_then(|p| syntax::extract_pattern_ident(&p))
                        .unwrap_or_else(|| "_".to_string());
                    let (type_name, access) = child
                        .field("type")
                        .map_or((None, ParamAccess::Unknown), |t| {
                            syntax::extract_base_type(&t)
                        });
                    if param_name == "self" {
                        self_receiver = Some(access);
                    } else {
                        params.push(ParamDecl {
                            name: param_name,
                            type_name,
                            access,
                        });
                    }
                }
            }
        }

        let return_type = node
            .field("return_type")
            .and_then(|t| syntax::extract_base_type(&t).0);

        let ExtractedBody {
            body_tree,
            field_accesses,
            calls,
            constructed_types,
            local_bindings,
            field_inits,
            referenced_idents,
            scoped_ident_refs,
        } = BodyWalker::new(self.tree).extract(node.field("body"));

        Some(FunctionDecl {
            name,
            owner_type,
            is_interface_impl,
            self_receiver,
            visibility,
            role,
            params,
            return_type,
            body_tree,
            field_accesses,
            calls,
            constructed_types,
            local_bindings,
            field_inits,
            referenced_idents,
            scoped_ident_refs,
            span: self.tree.node_span(node),
        })
    }
}

fn apply_rust_module_segment(buf: &mut Vec<String>, seg: &str) -> Option<()> {
    match seg {
        "self" => Some(()),
        "super" => {
            if buf.len() <= 1 {
                return None;
            }
            buf.pop();
            Some(())
        }
        other => {
            buf.push(other.to_string());
            Some(())
        }
    }
}

fn extract_visibility(node: &TsNode<'_>) -> Visibility {
    for child in node.children() {
        if child.kind() == "visibility_modifier" {
            let text = child.text();
            if text.contains("crate") || text.contains("super") || text.contains("in ") {
                return Visibility::Internal;
            }
            return Visibility::Public;
        }
    }
    Visibility::Private
}

fn extract_derives(attrs: &[String]) -> Vec<String> {
    let mut derives = Vec::new();
    for attr in attrs {
        if let Some((_, rest)) = attr.split_once("derive(")
            && let Some((inner, _)) = rest.split_once(')')
        {
            for item in inner.split(',') {
                let name = item.trim().rsplit("::").next().unwrap_or("").trim();
                if !name.is_empty() {
                    derives.push(name.to_string());
                }
            }
        }
    }
    derives
}
