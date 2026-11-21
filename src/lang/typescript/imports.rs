use super::TsNode;
use super::syntax;
use crate::ast::item::{ExportDecl, ImportDecl, ImportKind};
use crate::ast::{ModulePath, SyntaxTree};
use crate::diagnostic::Span;

pub(super) struct ImportBatch {
    pub(super) imports: Vec<ImportDecl>,
    pub(super) namespace_imports: Vec<(String, ModulePath)>,
}

pub(super) struct ExportClauseResult {
    pub(super) exports: Vec<ExportDecl>,
    pub(super) local_exported: Vec<(String, bool)>,
}

struct ImportSource {
    resolved_module: Option<ModulePath>,
    module_segments: Vec<String>,
    in_test_module: bool,
    span: Span,
}

impl ImportSource {
    fn to_decl(&self, kind: ImportKind, symbol: String, alias: Option<String>) -> ImportDecl {
        ImportDecl {
            resolved_module: self.resolved_module.clone(),
            module_segments: self.module_segments.clone(),
            kind,
            symbol,
            alias,
            in_test_module: self.in_test_module,
            span: self.span.clone(),
        }
    }

    fn collect_named(&self, imports_node: &TsNode<'_>, out: &mut Vec<ImportDecl>) {
        for spec in imports_node.children() {
            if spec.kind() != "import_specifier" {
                continue;
            }
            let Some(name_node) = spec.field("name") else {
                continue;
            };
            let raw_name = syntax::unquote_string(&name_node.text());
            let alias = spec.field("alias").map(|a| a.text().into_owned());
            if raw_name == "default" {
                let local = alias.unwrap_or_else(|| String::from("default"));
                out.push(self.to_decl(ImportKind::Default, local, None));
            } else {
                out.push(self.to_decl(ImportKind::Named, raw_name, alias));
            }
        }
    }
}

pub(super) struct ModuleImportParser<'a> {
    tree: &'a SyntaxTree,
    base_dir: &'a [String],
}

impl<'a> ModuleImportParser<'a> {
    pub(super) fn new(tree: &'a SyntaxTree, base_dir: &'a [String]) -> Self {
        Self { tree, base_dir }
    }

    pub(super) fn parse_import(&self, node: &TsNode<'_>, in_test_file: bool) -> ImportBatch {
        let mut batch = ImportBatch {
            imports: Vec::new(),
            namespace_imports: Vec::new(),
        };
        let Some(source_node) = node.field("source") else {
            return batch;
        };
        let specifier = syntax::unquote_string(&source_node.text());
        let resolved_module = syntax::resolve_relative_module_path(self.base_dir, &specifier);
        let module_segments = resolved_module
            .as_ref()
            .map_or_else(Vec::new, |resolved| resolved.segments().to_vec());
        let src = ImportSource {
            resolved_module,
            module_segments,
            in_test_module: in_test_file,
            span: self.tree.node_span(node),
        };

        let Some(clause) = node.children().find(|c| c.kind() == "import_clause") else {
            return batch;
        };

        for part in clause.children() {
            match part.kind().as_ref() {
                "identifier" => {
                    batch.imports.push(src.to_decl(
                        ImportKind::Default,
                        part.text().into_owned(),
                        None,
                    ));
                }
                "namespace_import" => {
                    if let Some(ident) = part.children().find(|c| c.kind() == "identifier") {
                        let ns_name = ident.text().into_owned();
                        if let Some(ref resolved) = src.resolved_module {
                            batch
                                .namespace_imports
                                .push((ns_name.clone(), resolved.clone()));
                        }
                        batch
                            .imports
                            .push(src.to_decl(ImportKind::Namespace, ns_name, None));
                    }
                }
                "named_imports" => src.collect_named(&part, &mut batch.imports),
                _ => {}
            }
        }
        batch
    }

    pub(super) fn parse_reexport(&self, node: &TsNode<'_>) -> Option<ExportClauseResult> {
        let source_field = node.field("source");
        let has_source = source_field.is_some();
        let source_module = source_field.and_then(|s| {
            let spec = syntax::unquote_string(&s.text());
            syntax::resolve_relative_module_path(self.base_dir, &spec)
        });

        if let Some(clause) = node.children().find(|c| c.kind() == "export_clause") {
            let mut exports = Vec::new();
            let mut local_exported = Vec::new();
            if has_source && source_module.is_none() {
                return Some(ExportClauseResult {
                    exports,
                    local_exported,
                });
            }
            for spec in clause.children() {
                if spec.kind() != "export_specifier" {
                    continue;
                }
                let Some(name_node) = spec.field("name") else {
                    continue;
                };
                let raw_name = syntax::unquote_string(&name_node.text());
                let exported_name = spec
                    .field("alias")
                    .map_or_else(|| raw_name.clone(), |a| syntax::unquote_string(&a.text()));
                let is_default = exported_name == "default";
                if !has_source {
                    local_exported.push((raw_name.clone(), is_default));
                }
                exports.push(ExportDecl {
                    source_module: source_module.clone(),
                    symbol: raw_name,
                    exported_name,
                    is_default,
                    is_wildcard: false,
                });
            }
            return Some(ExportClauseResult {
                exports,
                local_exported,
            });
        }

        if let Some(ns_export) = node.children().find(|c| c.kind() == "namespace_export") {
            let exported_name = ns_export
                .children()
                .find(TsNode::is_named)
                .map(|n| syntax::unquote_string(&n.text()))
                .unwrap_or_default();
            let exports = if let Some(src_mod) = source_module
                && !exported_name.is_empty()
            {
                vec![ExportDecl {
                    source_module: Some(src_mod),
                    symbol: String::new(),
                    exported_name,
                    is_default: false,
                    is_wildcard: true,
                }]
            } else {
                Vec::new()
            };
            return Some(ExportClauseResult {
                exports,
                local_exported: Vec::new(),
            });
        }

        if has_source && syntax::has_modifier_token(node, "*") {
            let exports = source_module.map_or_else(Vec::new, |src_mod| {
                vec![ExportDecl {
                    source_module: Some(src_mod),
                    symbol: String::new(),
                    exported_name: String::new(),
                    is_default: false,
                    is_wildcard: true,
                }]
            });
            return Some(ExportClauseResult {
                exports,
                local_exported: Vec::new(),
            });
        }

        None
    }
}
