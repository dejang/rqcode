use super::TsNode;
use super::conventions::TypeScriptConventions;
use super::coupling;
use super::syntax;
use crate::ast::item::{
    ExportDecl, ExportOrigin, ImportDecl, ImportKind, ImportOrigin, Visibility,
};
use crate::ast::{BarrelKind, ModulePath, SyntaxTree};
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
    origin: ImportOrigin,
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
            bypasses_visibility: false,
            origin: self.origin,
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
    conventions: &'a TypeScriptConventions,
}

impl<'a> ModuleImportParser<'a> {
    pub(super) fn new(
        tree: &'a SyntaxTree,
        base_dir: &'a [String],
        conventions: &'a TypeScriptConventions,
    ) -> Self {
        Self {
            tree,
            base_dir,
            conventions,
        }
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
        let is_asset = self.conventions.is_asset_specifier(&specifier);
        let resolved_module = if is_asset {
            None
        } else {
            syntax::resolve_relative_module_path(self.base_dir, &specifier)
        };
        let origin = if is_asset {
            ImportOrigin::StandardValue
        } else {
            coupling::classify_import_origin(&specifier, resolved_module.as_ref(), self.conventions)
        };
        let module_segments = resolved_module
            .as_ref()
            .map_or_else(Vec::new, |resolved| resolved.segments().to_vec());
        let src = ImportSource {
            resolved_module,
            module_segments,
            in_test_module: in_test_file,
            origin,
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
        let span = self.tree.node_span(node);
        let source_field = node.field("source");
        let has_source = source_field.is_some();
        let source_module = source_field.and_then(|s| {
            let spec = syntax::unquote_string(&s.text());
            if self.conventions.is_asset_specifier(&spec) {
                return None;
            }
            syntax::resolve_relative_module_path(self.base_dir, &spec)
        });
        let source_origin = if !has_source {
            ExportOrigin::Local
        } else if source_module.is_some() {
            ExportOrigin::DirectInternal
        } else {
            ExportOrigin::DirectExternal
        };

        if let Some(clause) = node.children().find(|c| c.kind() == "export_clause") {
            let mut exports = Vec::new();
            let mut local_exported = Vec::new();
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
                    visibility: Visibility::Public,
                    origin: source_origin,
                    span: span.clone(),
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
            let exports = if has_source && !exported_name.is_empty() {
                vec![ExportDecl {
                    source_module,
                    symbol: String::new(),
                    exported_name,
                    is_default: false,
                    is_wildcard: true,
                    visibility: Visibility::Public,
                    origin: source_origin,
                    span,
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
            return Some(ExportClauseResult {
                exports: vec![ExportDecl {
                    source_module,
                    symbol: String::new(),
                    exported_name: String::new(),
                    is_default: false,
                    is_wildcard: true,
                    visibility: Visibility::Public,
                    origin: source_origin,
                    span,
                }],
                local_exported: Vec::new(),
            });
        }

        None
    }
}

pub(super) fn classify_barrel_kind(
    tree: &SyntaxTree,
    imports: &[ImportDecl],
    exports: &[ExportDecl],
    conventions: &TypeScriptConventions,
) -> BarrelKind {
    let rel_path = tree.relative_path();
    if rel_path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| conventions.is_ambient_declaration(n))
        || !exports.iter().any(ExportDecl::is_reexport)
    {
        return BarrelKind::None;
    }
    if rel_path
        .file_stem()
        .and_then(|s| s.to_str())
        .is_some_and(|s| conventions.library_root_stems.iter().any(|r| r == s))
    {
        return BarrelKind::DirectoryIndex;
    }
    let imported_locals: std::collections::HashSet<&str> = imports
        .iter()
        .filter(|imp| !imp.in_test_module)
        .map(ImportDecl::local_name)
        .collect();
    for child in tree.root().children() {
        if !child.is_named() || child.kind() == "comment" || child.kind() == "import_statement" {
            continue;
        }
        if child.kind() != "export_statement" || child.field("declaration").is_some() {
            return BarrelKind::None;
        }
        if child.field("source").is_some() {
            continue;
        }
        if let Some(clause) = child.children().find(|c| c.kind() == "export_clause") {
            let mut count = 0usize;
            for spec in clause.children() {
                if spec.kind() != "export_specifier" {
                    continue;
                }
                let Some(name_node) = spec.field("name") else {
                    return BarrelKind::None;
                };
                if !imported_locals.contains(syntax::unquote_string(&name_node.text()).as_str()) {
                    return BarrelKind::None;
                }
                count += 1;
            }
            if count == 0 {
                return BarrelKind::None;
            }
            continue;
        }
        let is_imported_default = child.field("value").is_some_and(|val| {
            syntax::has_modifier_token(&child, "default")
                && val.kind() == "identifier"
                && imported_locals.contains(val.text().as_ref())
        });
        if !is_imported_default {
            return BarrelKind::None;
        }
    }
    BarrelKind::PureReexport
}

#[cfg(test)]
mod tests {
    use crate::Analyzer;
    use crate::config::Config;

    fn run_tc_project(files: &[(&str, &str)]) -> crate::Report {
        let dir = tempfile::tempdir().expect("tempdir");
        for (rel, body) in files {
            let full = dir.path().join(rel);
            std::fs::create_dir_all(full.parent().expect("parent")).expect("mkdir");
            std::fs::write(&full, body).expect("write");
        }
        let cfg = Config::load_from_dir(dir.path()).expect("config");
        Analyzer::from_config(&cfg, dir.path())
            .expect("analyzer")
            .check_path(dir.path(), &cfg)
            .expect("check")
    }

    #[test]
    fn ambiguous_alias_named_import_remains_reachable() {
        let tc_only = "[cohesion]\nenabled = false\n[similarity]\nenabled = false\n[ast_grep]\nenabled = false\n";
        let report = run_tc_project(&[
            ("rqcode.toml", tc_only),
            (
                "src/main.tsx",
                "import { Row } from '@/components/table/Row';\nexport function App() { return <Row />; }\ncreateRoot(rootEl).render(<App />);\n",
            ),
            (
                "src/components/table/Row.tsx",
                "export function Row() { return <tr><td>table</td></tr>; }\n",
            ),
            (
                "src/components/list/Row.tsx",
                "export function Row() { return <li>list</li>; }\n",
            ),
            (
                "src/components/table/Row.test.tsx",
                "import { describe, expect, it } from 'vitest';\nimport { Row } from './Row';\ndescribe('Row', () => { it('renders', () => { expect(Row()).toBeTruthy(); }); });\n",
            ),
        ]);
        assert!(
            report.diagnostics().is_empty(),
            "ambiguous @/ import must preserve reachability:\n{}",
            report.render_text()
        );
    }

    #[test]
    fn namespace_reexport_does_not_resolve_arbitrary_named_imports() {
        let tc_only = "[cohesion]\nenabled = false\n[similarity]\nenabled = false\n[ast_grep]\nenabled = false\n";
        let report = run_tc_project(&[
            ("rqcode.toml", tc_only),
            (
                "src/main.tsx",
                "import { DeadHelper } from './barrel';\nconsole.log(DeadHelper);\n",
            ),
            ("src/barrel.ts", "export * as UI from './ui';\n"),
            ("src/ui.tsx", "export function DeadHelper() { return 1; }\n"),
            (
                "src/alt.tsx",
                "export function DeadHelper() { return 2; }\n",
            ),
            (
                "src/ui.test.tsx",
                "import { describe, expect, it } from 'vitest';\nimport { DeadHelper } from './ui';\ndescribe('ui', () => { it('calls dead helper', () => { expect(DeadHelper()).toBe(1); }); });\n",
            ),
        ]);
        assert!(
            report
                .diagnostics()
                .iter()
                .any(|d| d.rule_id() == "test-only-production-api"
                    && d.message().contains("DeadHelper")),
            "export * as UI must not delegate arbitrary named imports:\n{}",
            report.render_text()
        );
    }

    #[test]
    fn deep_barrel_and_renamed_local_export_resolve_cleanly() {
        let tc_only = "[cohesion]\nenabled = false\n[similarity]\nenabled = false\n[ast_grep]\nenabled = false\n";
        let report = run_tc_project(&[
            ("rqcode.toml", tc_only),
            (
                "src/main.tsx",
                "import { DeepHelper } from './b1';\nimport { PublicAlias } from './renamed';\nconsole.log(DeepHelper(), PublicAlias());\n",
            ),
            ("src/b1.ts", "export { DeepHelper } from './b2';\n"),
            ("src/b2.ts", "export { DeepHelper } from './b3';\n"),
            ("src/b3.ts", "export { DeepHelper } from './b4';\n"),
            ("src/b4.ts", "export { DeepHelper } from './b5';\n"),
            ("src/b5.ts", "export { DeepHelper } from './leaf';\n"),
            (
                "src/leaf.ts",
                "export function DeepHelper() { return 1; }\n",
            ),
            (
                "src/renamed.ts",
                "function InternalImpl() { return 2; }\nexport { InternalImpl as PublicAlias };\n",
            ),
            (
                "src/leaf.test.ts",
                "import { describe, expect, it } from 'vitest';\nimport { DeepHelper } from './leaf';\nimport { PublicAlias } from './renamed';\ndescribe('exports', () => { it('runs', () => { expect(DeepHelper()).toBe(1); expect(PublicAlias()).toBe(2); }); });\n",
            ),
        ]);
        assert!(
            report.diagnostics().is_empty(),
            "deep >4-hop barrel and renamed local export must stay reachable:\n{}",
            report.render_text()
        );
    }
}
