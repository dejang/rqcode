use super::TsNode;
use super::conventions::TypeScriptConventions;
use super::expr::TsBodyContext;
use super::members::DeclMeta;
use super::props;
use super::syntax;
use crate::ast::item::{
    ExportBinding, ExportDecl, ExportOrigin, FunctionDecl, FunctionRole, ImportDecl, ImportKind,
    ImportOrigin, TypeDecl, TypeRef, Visibility,
};
use crate::ast::{FileRole, ModulePath, SyntaxTree};
use crate::diagnostic::Span;
use crate::lang::common::is_comment_kind;
use std::collections::HashSet;
use std::ops::Range;
use std::path::Component;

/// Signature metadata for recording a top-level or test callable in [`super::extract::TypeScriptExtractor`].
pub(super) struct CallableSignature {
    pub(super) name: String,
    pub(super) meta: DeclMeta,
    pub(super) role: FunctionRole,
    pub(super) return_type: Option<TypeRef>,
}

pub(super) struct PendingExportCall<'a> {
    pub(super) call_node: TsNode<'a>,
    pub(super) span: Span,
    pub(super) in_test_context: bool,
}

pub(super) fn mark_exported_types(types: &mut [TypeDecl], symbol: &str, is_default: bool) {
    for ty in types {
        if ty.name == symbol {
            ty.visibility = Visibility::Public;
            if is_default {
                ty.export_binding = ExportBinding::Default;
            }
        }
    }
}

pub(super) fn mark_exported_functions(
    functions: &mut [FunctionDecl],
    symbol: &str,
    is_default: bool,
) {
    for func in functions {
        if func.owner_type.is_none() && func.name == symbol {
            func.visibility = Visibility::Public;
            if is_default {
                func.export_binding = ExportBinding::Default;
            }
        }
    }
}

pub(super) fn reclassify_imported_exports(imports: &[ImportDecl], exports: &mut [ExportDecl]) {
    let imported_locals: HashSet<&str> = imports
        .iter()
        .filter(|imp| !imp.in_test_module)
        .map(ImportDecl::local_name)
        .collect();
    for exp in exports {
        if exp.origin == ExportOrigin::Local && imported_locals.contains(exp.symbol.as_str()) {
            exp.origin = ExportOrigin::ImportedBinding;
        }
    }
}

pub(super) fn base_dir_segments(tree: &SyntaxTree) -> Vec<String> {
    tree.relative_path().parent().map_or_else(Vec::new, |dir| {
        dir.components()
            .filter_map(|c| match c {
                Component::Normal(seg) => seg.to_str().map(String::from),
                _ => None,
            })
            .collect()
    })
}

/// Unwrapped inner component function produced from `memo(...)` / `forwardRef(...)`.
pub(super) struct UnwrappedComponent<'a> {
    pub(super) func_node: TsNode<'a>,
    pub(super) prop_type_hint: Option<TsNode<'a>>,
    pub(super) extra_args: Vec<TsNode<'a>>,
}

pub(super) fn unwrap_component_wrapper<'a>(
    call_node: &TsNode<'a>,
    conventions: &TypeScriptConventions,
) -> Option<UnwrappedComponent<'a>> {
    let wrapper_name = extract_call_simple_name(call_node)?;
    if !conventions.is_component_wrapper(&wrapper_name) {
        return None;
    }
    let args = call_node.field("arguments")?;
    let mut named_args = args
        .children()
        .filter(|c| c.is_named() && !is_comment_kind(c.kind().as_ref()));
    let first_arg = named_args.next()?;
    let mut extra_args: Vec<TsNode<'a>> = named_args.collect();
    let outer_hint = props::extract_wrapper_call_prop_hint(call_node, &wrapper_name, conventions);

    match first_arg.kind().as_ref() {
        "arrow_function" | "function_expression" => Some(UnwrappedComponent {
            func_node: first_arg,
            prop_type_hint: outer_hint,
            extra_args,
        }),
        "call_expression" => {
            let mut inner = unwrap_component_wrapper(&first_arg, conventions)?;
            inner.extra_args.append(&mut extra_args);
            Some(UnwrappedComponent {
                func_node: inner.func_node,
                prop_type_hint: inner.prop_type_hint.or(outer_hint),
                extra_args: inner.extra_args,
            })
        }
        _ => None,
    }
}

pub(super) fn extract_lazy_import_target(
    call_node: &TsNode<'_>,
    ctx: &TsBodyContext<'_, '_>,
) -> Option<ModulePath> {
    let callee_name = extract_call_simple_name(call_node)?;
    if !ctx.conventions().is_lazy_wrapper(callee_name.as_str()) {
        return None;
    }
    let args = call_node.field("arguments")?;
    let first_arg = args
        .children()
        .find(|c| c.is_named() && !is_comment_kind(c.kind().as_ref()))?;
    if !matches!(
        first_arg.kind().as_ref(),
        "arrow_function" | "function_expression"
    ) {
        return None;
    }
    let body = first_arg.field("body")?;
    find_first_dynamic_import(&body, ctx)
}

pub(super) fn collect_top_level_dynamic_imports(
    node: &TsNode<'_>,
    ctx: &TsBodyContext<'_, '_>,
) -> Vec<(ImportDecl, String)> {
    let mut out = Vec::new();
    collect_top_level_dynamic_imports_into(node, ctx, &mut out);
    out
}

pub(super) fn build_lazy_import_and_export(
    name: String,
    target_mod: &ModulePath,
    visibility: Visibility,
    is_default_export: bool,
    span: Span,
) -> (ImportDecl, Option<ExportDecl>) {
    let import = ImportDecl {
        resolved_module: Some(target_mod.clone()),
        module_segments: target_mod.segments().to_vec(),
        kind: ImportKind::Default,
        symbol: name.clone(),
        alias: None,
        in_test_module: false,
        bypasses_visibility: false,
        origin: ImportOrigin::Project,
        span: span.clone(),
    };
    let export = (visibility == Visibility::Public).then(|| ExportDecl {
        source_module: None,
        symbol: name.clone(),
        exported_name: if is_default_export {
            String::from("default")
        } else {
            name
        },
        is_default: is_default_export,
        is_wildcard: false,
        visibility: Visibility::Public,
        origin: ExportOrigin::ImportedBinding,
        span,
    });
    (import, export)
}

pub(super) fn find_exported_call_target(
    call_node: &TsNode<'_>,
    local_names: &HashSet<String>,
) -> Option<(String, Range<usize>)> {
    let args = call_node.field("arguments")?;
    let first_arg = args
        .children()
        .find(|c| c.is_named() && !is_comment_kind(c.kind().as_ref()))?;
    match first_arg.kind().as_ref() {
        "identifier" => {
            let name = first_arg.text().into_owned();
            local_names
                .contains(&name)
                .then(|| (name, first_arg.range()))
        }
        "call_expression" => find_exported_call_target(&first_arg, local_names),
        _ => None,
    }
}

pub(super) fn local_default_export_decl(symbol: String, span: Span) -> ExportDecl {
    ExportDecl {
        source_module: None,
        symbol,
        exported_name: String::from("default"),
        is_default: true,
        is_wildcard: false,
        visibility: Visibility::Public,
        origin: ExportOrigin::Local,
        span,
    }
}

pub(super) fn is_type_only_export(node: &TsNode<'_>) -> bool {
    syntax::has_modifier_token(node, "type")
        || node
            .field("declaration")
            .is_some_and(|decl| decl.kind() == "type_alias_declaration")
}

/// Reclassify a CRA-style `src/index.ts(x)` with no exports and top-level side effects
/// from [`FileRole::LibraryRoot`] to [`FileRole::BinaryEntrypoint`].
pub(super) fn finalize_file_role(
    role: FileRole,
    has_exported_items: bool,
    has_top_level_idents: bool,
) -> FileRole {
    if role == FileRole::LibraryRoot && !has_exported_items && has_top_level_idents {
        FileRole::BinaryEntrypoint
    } else {
        role
    }
}

fn extract_call_simple_name(call_node: &TsNode<'_>) -> Option<String> {
    let func_node = call_node.field("function")?;
    match func_node.kind().as_ref() {
        "identifier" => Some(func_node.text().into_owned()),
        "member_expression" => func_node.field("property").map(|p| p.text().into_owned()),
        _ => None,
    }
}

fn find_first_dynamic_import(node: &TsNode<'_>, ctx: &TsBodyContext<'_, '_>) -> Option<ModulePath> {
    if node.kind() == "call_expression"
        && let Some(target) = ctx.extract_dynamic_import_target(node)
    {
        return Some(target);
    }
    for child in node.children() {
        if child.is_named()
            && let Some(target) = find_first_dynamic_import(&child, ctx)
        {
            return Some(target);
        }
    }
    None
}

fn collect_top_level_dynamic_imports_into(
    node: &TsNode<'_>,
    ctx: &TsBodyContext<'_, '_>,
    out: &mut Vec<(ImportDecl, String)>,
) {
    if node.kind() == "call_expression"
        && let Some(target_mod) = ctx.extract_dynamic_import_target(node)
    {
        let target_key = target_mod.to_string();
        let decl = ImportDecl {
            resolved_module: Some(target_mod.clone()),
            module_segments: target_mod.segments().to_vec(),
            kind: ImportKind::Default,
            symbol: target_key.clone(),
            alias: None,
            in_test_module: false,
            bypasses_visibility: false,
            origin: ImportOrigin::Project,
            span: ctx.tree().node_span(node),
        };
        out.push((decl, target_key));
        return;
    }
    for child in node.children() {
        if child.is_named() {
            collect_top_level_dynamic_imports_into(&child, ctx, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::ast::item::{CallKind, CallReceiver, TypeRef};
    use crate::ast::{FileRole, SyntaxTree};
    use crate::lang::LanguageFrontend;
    use crate::lang::typescript::{
        TypeScriptConventions, TypeScriptFrontend, tsx_language_spec, typescript_language_spec,
    };

    #[test]
    fn extracts_dynamic_import_inside_function_body_as_scoped_default_call() {
        let src = r#"
export async function loadSettings() {
    return import("./pages/Settings");
}
"#;
        let tree = SyntaxTree::parse("src/App.tsx", "src/App.tsx", tsx_language_spec(), src);
        let ast = TypeScriptFrontend::new().extract(&tree, std::path::Path::new("."));
        let func = ast
            .functions
            .iter()
            .find(|f| f.name == "loadSettings")
            .expect("loadSettings extracted");
        assert!(func.calls.iter().any(|c| {
            c.kind == CallKind::Scoped
                && c.callee == "default"
                && c.receiver == Some(CallReceiver::Named("src::pages::Settings".into()))
        }));
    }

    #[test]
    fn value_only_library_index_stays_library_root_while_cra_entry_becomes_binary() {
        let frontend = TypeScriptFrontend::new();
        let value_roots = [
            "export const theme = createTheme({ primary: '#000' });",
            "export const UI = { Button, Card };",
            "export default { Button, Card };",
            "export default createStore(reducer);",
        ];
        for src in value_roots {
            let tree = SyntaxTree::parse(
                "src/index.ts",
                "src/index.ts",
                typescript_language_spec(),
                src,
            );
            let ast = frontend.extract(&tree, std::path::Path::new("."));
            assert_eq!(
                ast.role,
                FileRole::LibraryRoot,
                "value-exporting index must remain LibraryRoot for `{src}`"
            );
        }

        let cra_src = "createRoot(rootEl).render(<App />);";
        let cra_tree = SyntaxTree::parse(
            "src/index.tsx",
            "src/index.tsx",
            tsx_language_spec(),
            cra_src,
        );
        let cra_ast = frontend.extract(&cra_tree, std::path::Path::new("."));
        assert_eq!(cra_ast.role, FileRole::BinaryEntrypoint);
    }

    #[test]
    fn custom_ref_forwarding_wrapper_uses_second_type_argument_for_props() {
        let mut conv = TypeScriptConventions::default();
        conv.component_wrapper_functions
            .push(String::from("customForwardRef"));
        conv.ref_forwarding_wrappers
            .push(String::from("customForwardRef"));
        let frontend = TypeScriptFrontend::with_conventions(conv);

        let src = r"
interface InputProps {
    order: Order;
}
export const OrderInput = customForwardRef<HTMLInputElement, InputProps>(({ order }, ref) => {
    return <input ref={ref} value={order.id} />;
});
";
        let tree = SyntaxTree::parse(
            "src/OrderInput.tsx",
            "src/OrderInput.tsx",
            tsx_language_spec(),
            src,
        );
        let ast = frontend.extract(&tree, std::path::Path::new("."));
        let func = ast
            .functions
            .iter()
            .find(|f| f.name == "OrderInput")
            .expect("OrderInput extracted");
        assert!(
            func.local_bindings.iter().any(|b| {
                b.name == "order" && b.type_name == Some(TypeRef::Named("Order".into()))
            })
        );
    }
}
