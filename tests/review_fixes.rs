use rqcode::ast::codebase::Codebase;
use rqcode::ast::item::{
    FieldDecl, FieldInit, FunctionDecl, FunctionRole, TypeDecl, TypeKind, TypeRef, Visibility,
};
use rqcode::ast::similarity::NormalizedTree;
use rqcode::ast::{FileAst, FileRole, ModulePath, SyntaxTree};
use rqcode::config::Config;
use rqcode::diagnostic::Span;
use rqcode::lang::LanguageFrontend;
use rqcode::lang::rust::{RustFrontend, rust_language_spec};
use rqcode::lang::typescript::{TypeScriptFrontend, typescript_language_spec};
use rqcode::rules::Rule;
use rqcode::rules::cohesion::CohesionRule;
use rqcode::rules::test_coupling::TestCouplingRule;
use std::error::Error;
use std::path::Path;

fn parse_ts(rel_path: &str, src: &str) -> (SyntaxTree, FileAst) {
    let tree = SyntaxTree::parse(rel_path, rel_path, typescript_language_spec(), src);
    let ast = TypeScriptFrontend.extract(&tree, Path::new("."));
    (tree, ast)
}

fn parse_rust(rel_path: &str, src: &str) -> (SyntaxTree, FileAst) {
    let tree = SyntaxTree::parse(rel_path, rel_path, rust_language_spec(), src);
    let ast = RustFrontend.extract(&tree, Path::new("."));
    (tree, ast)
}

#[test]
fn non_rust_language_uses_generic_field_init_noun_in_test_coupling() {
    let ts_spec = typescript_language_spec();
    let tree = SyntaxTree::parse("src/vault.ts", "src/vault.ts", ts_spec.clone(), "");
    let ast = FileAst {
        path: "src/vault.ts".into(),
        relative_path: "src/vault.ts".into(),
        module_path: ModulePath::new(vec!["src".into(), "vault".into()]),
        language: ts_spec.id(),
        member_separator: ts_spec.member_separator(),
        role: FileRole::Module,
        imports: Vec::new(),
        exports: Vec::new(),
        types: vec![TypeDecl {
            name: "Vault".into(),
            kind: TypeKind::Class,
            visibility: Visibility::Public,
            in_test_module: false,
            is_default_export: false,
            fields: vec![FieldDecl {
                name: "secret".into(),
                type_name: None,
                visibility: Visibility::Private,
                is_mut_ref: false,
                span: Span::new("src/vault.ts", 2, 1, 2, 20),
            }],
            annotations: Vec::new(),
            span: Span::new("src/vault.ts", 1, 1, 3, 2),
        }],
        functions: vec![FunctionDecl {
            name: "constructs_vault".into(),
            owner_type: None,
            is_interface_impl: false,
            self_receiver: None,
            visibility: Visibility::Private,
            role: FunctionRole::Test,
            params: Vec::new(),
            return_type: None,
            body_tree: NormalizedTree::default(),
            field_accesses: Vec::new(),
            calls: Vec::new(),
            constructed_types: vec![TypeRef::Named("Vault".into())],
            local_bindings: Vec::new(),
            field_inits: vec![FieldInit {
                type_name: TypeRef::Named("Vault".into()),
                field_name: "secret".into(),
                span: Span::new("src/vault.ts", 6, 5, 6, 20),
            }],
            referenced_idents: Vec::new(),
            scoped_ident_refs: Vec::new(),
            span: Span::new("src/vault.ts", 5, 1, 7, 2),
        }],
        top_level_idents: Vec::new(),
        suppressions: Vec::new(),
    };

    let codebase = Codebase::build(vec![tree], vec![ast]);
    let diags = TestCouplingRule.check(&codebase, &Config::default());
    assert!(
        diags
            .iter()
            .any(|d| d.rule_id() == "test-accesses-private-field"
                && d.message().contains("initializes via field initializer")),
        "expected language-agnostic field initializer wording for non-Rust grammar, got: {diags:?}"
    );
    assert!(
        !diags.iter().any(|d| d.message().contains("struct literal")),
        "non-Rust diagnostic must not mention Rust struct literals: {diags:?}"
    );
}

#[test]
fn typescript_class_decorators_populate_type_annotations() {
    let (_, ast) = parse_ts(
        "src/service.ts",
        r"
@Injectable()
export class BillingService {}

@Entity
class LedgerRow {}
",
    );
    assert_eq!(ast.types.len(), 2);
    assert!(ast.types[0].has_annotation("Injectable"));
    assert!(ast.types[1].has_annotation("Entity"));
}

#[test]
fn relative_imports_from_index_test_spec_and_index_dir_resolve_accurately() {
    let (_, test_ast) = parse_ts(
        "src/models/index.test.ts",
        "import { Order } from './order';\nimport * as selfBarrel from '.';\n",
    );
    assert_eq!(
        test_ast.imports[0].resolved_module,
        Some(ModulePath::new(vec![
            "src".into(),
            "models".into(),
            "order".into()
        ]))
    );
    assert_eq!(
        test_ast.imports[1].resolved_module,
        Some(ModulePath::new(vec!["src".into(), "models".into()]))
    );

    let (_, spec_ast) = parse_ts(
        "src/models/index.spec.ts",
        "import { Order } from './order';\n",
    );
    assert_eq!(
        spec_ast.imports[0].resolved_module,
        Some(ModulePath::new(vec![
            "src".into(),
            "models".into(),
            "order".into()
        ]))
    );

    let (_, nested_index_dir) = parse_ts(
        "index/index.ts",
        "import { Order } from './order';\nimport { Shared } from '../shared';\n",
    );
    assert_eq!(
        nested_index_dir.imports[0].resolved_module,
        Some(ModulePath::new(vec!["index".into(), "order".into()]))
    );
    assert_eq!(
        nested_index_dir.imports[1].resolved_module,
        Some(ModulePath::new(vec!["shared".into()]))
    );

    let (_, root_index) = parse_ts("index.ts", "import { Order } from './order';\n");
    assert_eq!(
        root_index.imports[0].resolved_module,
        Some(ModulePath::new(vec!["order".into()]))
    );
}

#[test]
fn parameter_named_readonly_and_method_named_static_are_not_misclassified()
-> Result<(), Box<dyn Error>> {
    let (tree, ast) = parse_ts(
        "src/router.ts",
        r"
export class Router {
    constructor(
        readonly: boolean,
        readonly id: string,
        override tag: string,
        override: number,
    ) {}

    static(path: string): string {
        return path;
    }

    static create(): Router {
        return new Router(false, 'a', 'b', 1);
    }
}
",
    );
    let field_names: Vec<&str> = ast.types[0]
        .fields
        .iter()
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(field_names, vec!["id", "tag"]);

    let static_method = ast
        .functions
        .iter()
        .find(|f| f.name == "static")
        .ok_or("missing instance method named static")?;
    assert!(
        static_method.has_self_receiver(),
        "instance method named `static` must retain self_receiver"
    );

    let create_method = ast
        .functions
        .iter()
        .find(|f| f.name == "create")
        .ok_or("missing static create method")?;
    assert!(!create_method.has_self_receiver());

    let codebase = Codebase::build(vec![tree], vec![ast]);
    let diags = CohesionRule.check(&codebase, &Config::default());
    assert!(
        !diags.iter().any(
            |d| d.rule_id() == "stateless-impl-method" && d.message().contains("Router.static")
        ),
        "instance method named `static` must not trigger stateless-impl-method: {diags:?}"
    );
    Ok(())
}

#[test]
fn external_reexports_do_not_expose_local_private_items_or_alias_them() {
    let (t1, f1) = parse_ts(
        "src/logger.ts",
        r"
class Logger {
    private secret: number = 1;
}

export { Logger } from 'third-party-pkg';
export { Logger as default } from 'third-party-pkg';
export { Logger as ExtLogger } from 'third-party-pkg';
export * as extNs from 'third-party-pkg';
export * from 'third-party-pkg';
",
    );
    assert_eq!(f1.types[0].visibility, Visibility::Private);
    assert!(!f1.types[0].is_default_export);
    assert_eq!(f1.exports, Vec::new());

    let (t2, f2) = parse_ts(
        "src/consumer.ts",
        r"
import { ExtLogger } from './logger';
export function useLogger(x: ExtLogger): void {}
",
    );
    let codebase = Codebase::build(vec![t1, t2], vec![f1, f2]);
    assert_eq!(
        codebase.functions()[0].resolved_params[0].resolved_type,
        None
    );

    let (rt1, rf1) = parse_rust(
        "src/logger.rs",
        "struct Logger { secret: u64 }\npub use third_party::Logger as ExtLogger;\n",
    );
    assert_eq!(rf1.exports, Vec::new());
    let (rt2, rf2) = parse_rust(
        "src/consumer.rs",
        "use crate::logger::ExtLogger;\npub fn use_logger(_x: &ExtLogger) {}\n",
    );
    let rust_cb = Codebase::build(vec![rt1, rt2], vec![rf1, rf2]);
    assert_eq!(
        rust_cb.functions()[0].resolved_params[0].resolved_type,
        None
    );
}

#[test]
fn production_calls_on_test_or_context_are_not_tests_and_parameterized_ts_tests_are() {
    let (_, prod_ast) = parse_ts(
        "src/app.ts",
        r#"
function cleanup(): void {}
function verify(): void {}
test.on("done", () => { cleanup(); });
context.use("auth", () => { verify(); });
"#,
    );
    assert!(
        prod_ast.functions.iter().all(|f| !f.is_test_code()),
        "production calls on test/context must not be classified as tests: {:?}",
        prod_ast.functions
    );
    assert!(prod_ast.top_level_idents.contains(&"cleanup".to_string()));
    assert!(prod_ast.top_level_idents.contains(&"verify".to_string()));

    let (_, test_ast) = parse_ts(
        "src/app.test.ts",
        r#"
test.on("done", () => {});
describe.each([1, 2])("suite %s", () => {
    it.concurrent.only("chained case", () => {});
    it.each([10])("each case %d", () => {});
    test.skipIf(false)("conditional case", () => {});
});
"#,
    );
    let test_names: Vec<&str> = test_ast
        .functions
        .iter()
        .filter(|f| f.is_test())
        .map(|f| f.name.as_str())
        .collect();
    assert_eq!(
        test_names,
        vec!["chained case", "each case %d", "conditional case"]
    );
}

#[test]
fn resolves_namespace_reexports_and_import_then_export_barrels() -> Result<(), Box<dyn Error>> {
    let (t_order, f_order) = parse_ts(
        "src/models/order.ts",
        r"
export default class Order {
    public total: number = 0;
    static create(): Order { return new Order(); }
}
",
    );
    let (t_legacy, f_legacy) = parse_ts(
        "src/legacy/order.ts",
        "export default class Order { public total: number = 0; }\n",
    );
    let (t_models_idx, f_models_idx) = parse_ts(
        "src/models/index.ts",
        r"
import { Order } from './order';
export { Order };
export default Order;
",
    );
    let (t_barrel, f_barrel) = parse_ts("src/barrel.ts", "export * as models from './models';\n");
    let (t_consumer, f_consumer) = parse_ts(
        "src/consumer.ts",
        r"
import { models } from './barrel';
import DefaultOrder, { Order as NamedOrder } from './models';

export function useAll(a: models.Order, b: NamedOrder, c: DefaultOrder): void {
    const built = models.Order.create();
    a.total += b.total + c.total + built.total;
}
",
    );

    let codebase = Codebase::build(
        vec![t_order, t_legacy, t_models_idx, t_barrel, t_consumer],
        vec![f_order, f_legacy, f_models_idx, f_barrel, f_consumer],
    );
    let models_order = codebase.types()[0].id;
    let use_all = codebase
        .functions()
        .iter()
        .find(|f| f.decl.name == "useAll")
        .ok_or("missing useAll function")?;

    for param in &use_all.resolved_params {
        assert_eq!(
            param.resolved_type,
            Some(models_order),
            "param `{}` should resolve to src::models::order Order",
            param.decl.name
        );
    }
    assert_eq!(
        use_all.resolved_local_types.get("built").copied(),
        Some(models_order)
    );

    // Cyclic import-then-export barrels must terminate cleanly without stack overflow.
    let (tc1, fc1) = parse_ts(
        "src/cyc_a.ts",
        "import { X } from './cyc_b';\nexport { X };\n",
    );
    let (tc2, fc2) = parse_ts(
        "src/cyc_b.ts",
        "import { X } from './cyc_a';\nexport { X };\n",
    );
    let (tc3, fc3) = parse_ts(
        "src/cyc_use.ts",
        "import { X } from './cyc_a';\nexport function f(x: X): void { void x; }\n",
    );
    let cyc_cb = Codebase::build(vec![tc1, tc2, tc3], vec![fc1, fc2, fc3]);
    assert_eq!(cyc_cb.functions()[0].resolved_params[0].resolved_type, None);
    Ok(())
}

#[test]
fn lowercase_method_call_does_not_falsely_infer_unrelated_free_function_return_type()
-> Result<(), Box<dyn Error>> {
    let (t_box, f_box) = parse_ts(
        "src/factory.ts",
        r"
export class SecretBox {
    private secret: number = 0;
    private reset(): void { this.secret = 0; }
}
export function create(): SecretBox {
    return new SecretBox();
}
",
    );
    let (t_use, f_use) = parse_ts(
        "src/caller.ts",
        r"
import * as factory from './factory';

export function run(repo: { create(): unknown }): void {
    const unrelated = repo.create();
    const fromNs = factory.create();
    void unrelated;
    void fromNs;
}
",
    );

    let codebase = Codebase::build(vec![t_box, t_use], vec![f_box, f_use]);
    let box_id = codebase.types()[0].id;
    let run_fn = codebase
        .functions()
        .iter()
        .find(|f| f.decl.name == "run")
        .ok_or("missing run function")?;

    assert_eq!(run_fn.resolved_local_types.get("unrelated"), None);
    assert_eq!(run_fn.resolved_local_types.get("fromNs"), Some(&box_id));
    Ok(())
}

#[test]
fn colocated_test_stub_does_not_overwrite_production_type() -> Result<(), Box<dyn Error>> {
    // Deliberately put the test file BEFORE the production file to verify order-independence.
    let (t_test, f_test) = parse_ts(
        "src/order.test.ts",
        "class Order { public mock: number = 0; }\n",
    );
    let (t_prod, f_prod) = parse_ts(
        "src/order.ts",
        r"
export class Order {
    private secret: number = 1;
    public run(): number { return this.secret; }
}
",
    );
    let (t_svc, f_svc) = parse_ts(
        "src/service.ts",
        "import { Order } from './order';\nexport function process(o: Order): number { return o.run(); }\n",
    );

    let codebase = Codebase::build(vec![t_test, t_prod, t_svc], vec![f_test, f_prod, f_svc]);
    let prod_order = codebase
        .types()
        .iter()
        .find(|t| !codebase.file(t.file_id).is_test_file())
        .ok_or("missing production Order")?
        .id;

    let run_method = codebase
        .functions()
        .iter()
        .find(|f| f.decl.name == "run")
        .ok_or("missing Order.run")?;
    assert_eq!(run_method.resolved_owner, Some(prod_order));

    let process_fn = codebase
        .functions()
        .iter()
        .find(|f| f.decl.name == "process")
        .ok_or("missing process")?;
    assert_eq!(
        process_fn.resolved_params[0].resolved_type,
        Some(prod_order)
    );

    // Rust `#[cfg(test)] mod tests` stub must also never overwrite production `struct Order`.
    let (rt, rf) = parse_rust(
        "src/order.rs",
        r"
pub struct Order { total: u64 }
impl Order { pub fn total(&self) -> u64 { self.total } }
#[cfg(test)]
mod tests {
    struct Order { mock: u64 }
}
",
    );
    let rust_cb = Codebase::build(vec![rt], vec![rf]);
    let rust_prod_order = rust_cb
        .types()
        .iter()
        .find(|t| !t.decl.in_test_module)
        .ok_or("missing rust prod Order")?
        .id;
    assert_eq!(rust_cb.functions()[0].resolved_owner, Some(rust_prod_order));
    Ok(())
}

#[test]
fn bare_external_default_import_does_not_match_internal_module() -> Result<(), Box<dyn Error>> {
    let (t_v1, f_v1) = parse_ts(
        "src/v1/order.ts",
        "export default class Order { public v: number = 1; }\n",
    );
    let (t_v2, f_v2) = parse_ts(
        "src/v2/order.ts",
        "export default class Order { public v: number = 2; }\n",
    );
    let (t_svc, f_svc) = parse_ts(
        "src/service.ts",
        "import order from 'order';\nexport function handle(x: order): void { void x; }\n",
    );

    let codebase = Codebase::build(vec![t_v1, t_v2, t_svc], vec![f_v1, f_v2, f_svc]);
    let handle = codebase
        .functions()
        .iter()
        .find(|f| f.decl.name == "handle")
        .ok_or("missing handle")?;
    assert_eq!(handle.resolved_params[0].resolved_type, None);
    Ok(())
}

#[test]
fn typescript_cli_app_with_nested_barrel_remains_binary_only_package() {
    let (t_main, f_main) = parse_ts("src/main.ts", "export function main(): void {}\n");
    let (t_barrel, f_barrel) = parse_ts("src/models/index.ts", "export * from './order';\n");
    let (t_order, f_order) = parse_ts("src/models/order.ts", "export class Order {}\n");

    let cli_cb = Codebase::build(
        vec![t_main, t_barrel, t_order],
        vec![f_main, f_barrel, f_order],
    );
    assert!(cli_cb.is_binary_only_package());

    let (t_main2, f_main2) = parse_ts("src/main.ts", "export function main(): void {}\n");
    let (t_root_idx, f_root_idx) = parse_ts("src/index.ts", "export * from './models';\n");
    let lib_cb = Codebase::build(vec![t_main2, t_root_idx], vec![f_main2, f_root_idx]);
    assert!(!lib_cb.is_binary_only_package());
}
