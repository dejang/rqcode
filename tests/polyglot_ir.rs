use rqcode::ast::codebase::Codebase;
use rqcode::ast::item::{
    ExportDecl, FunctionDecl, FunctionRole, ImportDecl, ImportKind, ParamAccess, ParamDecl,
    TypeDecl, TypeKind, TypeRef, Visibility,
};
use rqcode::ast::vector::NormalizedAst;
use rqcode::ast::{FileAst, FileRole, ModulePath, SyntaxTree};
use rqcode::config::Config;
use rqcode::diagnostic::Span;
use rqcode::lang::LanguageFrontend;
use rqcode::lang::rust::{RustFrontend, rust_language_spec};
use rqcode::rules::Rule;
use rqcode::rules::cohesion::CohesionRule;
use std::path::Path;

fn make_type(name: &str, file: &str, is_default_export: bool) -> TypeDecl {
    TypeDecl {
        name: name.to_string(),
        kind: TypeKind::Class,
        visibility: Visibility::Public,
        in_test_module: false,
        is_default_export,
        fields: Vec::new(),
        annotations: Vec::new(),
        span: Span::new(file, 1, 1, 3, 2),
    }
}

fn make_fn_with_param(
    name: &str,
    owner_type: Option<&str>,
    param_ty: &str,
    file: &str,
) -> FunctionDecl {
    FunctionDecl {
        name: name.to_string(),
        owner_type: owner_type.map(str::to_string),
        is_interface_impl: false,
        self_receiver: None,
        visibility: Visibility::Public,
        role: FunctionRole::Production,
        params: vec![ParamDecl {
            name: "x".to_string(),
            type_name: Some(TypeRef::Named(param_ty.to_string())),
            access: ParamAccess::Owned,
        }],
        return_type: None,
        body_tokens: NormalizedAst::default(),
        field_accesses: Vec::new(),
        calls: Vec::new(),
        constructed_types: Vec::new(),
        local_bindings: Vec::new(),
        field_inits: Vec::new(),
        referenced_idents: Vec::new(),
        scoped_ident_refs: Vec::new(),
        span: Span::new(file, 5, 1, 7, 2),
    }
}

fn make_import(module: &ModulePath, kind: ImportKind, symbol: &str, line: usize) -> ImportDecl {
    ImportDecl {
        resolved_module: Some(module.clone()),
        module_segments: module.segments().to_vec(),
        kind,
        symbol: symbol.to_string(),
        alias: None,
        in_test_module: false,
        span: Span::new("src/service.ts", line, 1, line, 40),
    }
}

fn make_file(rel: &str, module: &[&str], sep: &'static str) -> (SyntaxTree, FileAst) {
    let spec = rust_language_spec();
    let tree = SyntaxTree::parse(rel, rel, spec.clone(), "");
    let ast = FileAst {
        path: rel.into(),
        relative_path: rel.into(),
        module_path: ModulePath::new(module.iter().map(|s| (*s).to_string()).collect()),
        language: spec.id(),
        member_separator: sep,
        role: FileRole::Module,
        imports: Vec::new(),
        exports: Vec::new(),
        types: Vec::new(),
        functions: Vec::new(),
        top_level_idents: Vec::new(),
        suppressions: Vec::new(),
    };
    (tree, ast)
}

#[test]
fn resolves_default_namespace_and_barrel_reexports_and_formats_member_separator() {
    let order_mod = ModulePath::new(vec!["src".into(), "models".into(), "order".into()]);
    let legacy_mod = ModulePath::new(vec!["src".into(), "legacy".into(), "order".into()]);
    let barrel_mod = ModulePath::new(vec!["src".into(), "models".into()]);

    let (t1, mut f1) = make_file("src/models/order.ts", &["src", "models", "order"], ".");
    f1.types = vec![
        make_type("Helper", "src/models/order.ts", false),
        make_type("Order", "src/models/order.ts", true),
    ];

    let (t2, mut f2) = make_file("src/legacy/order.ts", &["src", "legacy", "order"], ".");
    f2.types = vec![make_type("Order", "src/legacy/order.ts", false)];

    let (t3, mut f3) = make_file("src/models/index.ts", &["src", "models"], ".");
    f3.exports = vec![
        ExportDecl {
            source_module: Some(order_mod.clone()),
            symbol: String::new(),
            exported_name: String::new(),
            is_default: false,
            is_wildcard: true,
        },
        ExportDecl {
            source_module: Some(order_mod.clone()),
            symbol: "Order".to_string(),
            exported_name: "CustomerOrder".to_string(),
            is_default: false,
            is_wildcard: false,
        },
        ExportDecl {
            source_module: Some(order_mod.clone()),
            symbol: "default".to_string(),
            exported_name: "default".to_string(),
            is_default: true,
            is_wildcard: false,
        },
    ];

    let (t4, mut f4) = make_file("src/service.ts", &["src", "service"], ".");
    f4.imports = vec![
        make_import(&order_mod, ImportKind::Default, "DefaultOrder", 1),
        make_import(&legacy_mod, ImportKind::Default, "LegacyOrder", 2),
        make_import(&barrel_mod, ImportKind::Default, "BarrelDefaultOrder", 3),
        make_import(&barrel_mod, ImportKind::Namespace, "models", 4),
        make_import(&barrel_mod, ImportKind::Named, "CustomerOrder", 5),
    ];
    f4.functions = vec![
        make_fn_with_param("via_default", None, "DefaultOrder", "src/service.ts"),
        make_fn_with_param(
            "via_fallback_default",
            None,
            "LegacyOrder",
            "src/service.ts",
        ),
        make_fn_with_param(
            "via_barrel_default",
            None,
            "BarrelDefaultOrder",
            "src/service.ts",
        ),
        make_fn_with_param("via_namespace", None, "models::Order", "src/service.ts"),
        make_fn_with_param("discount", Some("Order"), "CustomerOrder", "src/service.ts"),
    ];

    let codebase = Codebase::build(vec![t1, t2, t3, t4], vec![f1, f2, f3, f4]);
    let models_order_id = codebase.types()[1].id;
    let legacy_order_id = codebase.types()[2].id;
    assert_eq!(codebase.ty(models_order_id).decl.name, "Order");
    assert_eq!(codebase.ty(legacy_order_id).decl.name, "Order");

    assert_eq!(
        codebase.functions()[0].resolved_params[0].resolved_type,
        Some(models_order_id)
    );
    assert_eq!(
        codebase.functions()[1].resolved_params[0].resolved_type,
        Some(legacy_order_id)
    );
    assert_eq!(
        codebase.functions()[2].resolved_params[0].resolved_type,
        Some(models_order_id)
    );
    assert_eq!(
        codebase.functions()[3].resolved_params[0].resolved_type,
        Some(models_order_id)
    );
    assert_eq!(
        codebase.functions()[4].resolved_params[0].resolved_type,
        Some(models_order_id)
    );
    assert_eq!(codebase.functions()[4].display_name(), "Order.discount");

    let diags = CohesionRule.check(&codebase, &Config::default());
    assert!(
        diags
            .iter()
            .any(|d| d.rule_id() == "stateless-impl-method"
                && d.message().contains("`Order.discount`")),
        "expected dot-separated method name in diagnostic, got: {diags:?}"
    );
}

#[test]
fn cyclic_barrel_reexports_terminate_cleanly() {
    let mod_a = ModulePath::new(vec!["src".into(), "a".into()]);
    let mod_b = ModulePath::new(vec!["src".into(), "b".into()]);

    let (t1, mut f1) = make_file("src/a.ts", &["src", "a"], ".");
    f1.exports.push(ExportDecl {
        source_module: Some(mod_b.clone()),
        symbol: String::new(),
        exported_name: String::new(),
        is_default: false,
        is_wildcard: true,
    });

    let (t2, mut f2) = make_file("src/b.ts", &["src", "b"], ".");
    f2.exports.push(ExportDecl {
        source_module: Some(mod_a),
        symbol: String::new(),
        exported_name: String::new(),
        is_default: false,
        is_wildcard: true,
    });

    let (t3, mut f3) = make_file("src/c.ts", &["src", "c"], ".");
    f3.imports
        .push(make_import(&mod_b, ImportKind::Named, "Missing", 1));
    f3.functions.push(make_fn_with_param(
        "use_missing",
        None,
        "Missing",
        "src/c.ts",
    ));

    let codebase = Codebase::build(vec![t1, t2, t3], vec![f1, f2, f3]);
    assert_eq!(
        codebase.functions()[0].resolved_params[0].resolved_type,
        None
    );
}

#[test]
fn resolves_rust_pub_use_barrel_reexport_across_modules() {
    let files_src = [
        ("src/models/order.rs", "pub struct Order { pub total: u64 }"),
        ("src/legacy/order.rs", "pub struct Order { pub total: u64 }"),
        (
            "src/models/mod.rs",
            "pub use self::order::Order as CustomerOrder;",
        ),
        (
            "src/pricing.rs",
            "use crate::models::CustomerOrder;\npub fn total(o: &CustomerOrder) -> u64 { o.total }",
        ),
    ];

    let mut trees = Vec::new();
    let mut asts = Vec::new();
    for (rel, src) in files_src {
        let tree = SyntaxTree::parse(rel, rel, rust_language_spec(), src);
        let ast = RustFrontend.extract(&tree, Path::new("."));
        trees.push(tree);
        asts.push(ast);
    }

    let codebase = Codebase::build(trees, asts);
    let models_order = codebase.types()[0].id;
    assert_eq!(
        codebase.ty(models_order).module_path.to_string(),
        "crate::models::order"
    );
    assert_eq!(
        codebase.functions()[0].resolved_params[0].resolved_type,
        Some(models_order)
    );
}
