mod body;
mod extract;
mod imports;
mod syntax;

use self::extract::TypeScriptExtractor;
use crate::ast::{
    CommentStyle, FileAst, FileRole, LanguageGrammar, LanguageId, LanguageSpec, ModulePath,
    SyntaxTree,
};
use crate::lang::{LanguageFrontend, common};
use ast_grep_core::Node;
use ast_grep_core::language::Language;
use ast_grep_core::tree_sitter::{LanguageExt, StrDoc, TSLanguage};
use ast_grep_language::TypeScript as AstGrepTypeScript;
use std::borrow::Cow;
use std::path::{Component, Path};
use std::sync::Arc;

type TsNode<'a> = Node<'a, StrDoc<LanguageSpec>>;

struct TypeScriptGrammar;

impl LanguageGrammar for TypeScriptGrammar {
    fn id(&self) -> LanguageId {
        LanguageId::new("typescript")
    }

    fn aliases(&self) -> &'static [&'static str] {
        &["typescript", "ts"]
    }

    fn extensions(&self) -> &'static [&'static str] {
        &["ts"]
    }

    fn comment_style(&self) -> CommentStyle {
        CommentStyle::new("//", None, Some(("/*", "*/")), &[])
    }

    fn member_separator(&self) -> &'static str {
        "."
    }

    fn ts_language(&self) -> TSLanguage {
        AstGrepTypeScript.get_ts_language()
    }

    fn kind_to_id(&self, kind: &str) -> u16 {
        AstGrepTypeScript.kind_to_id(kind)
    }

    fn field_to_id(&self, field: &str) -> Option<u16> {
        AstGrepTypeScript.field_to_id(field)
    }

    fn expando_char(&self) -> char {
        AstGrepTypeScript.expando_char()
    }

    fn pre_process_pattern<'q>(&self, query: &'q str) -> Cow<'q, str> {
        AstGrepTypeScript.pre_process_pattern(query)
    }
}

/// Construct the [`LanguageSpec`] for TypeScript.
#[must_use]
pub fn typescript_language_spec() -> LanguageSpec {
    LanguageSpec::new(Arc::new(TypeScriptGrammar))
}

pub struct TypeScriptFrontend;

impl LanguageFrontend for TypeScriptFrontend {
    fn grammar(&self) -> LanguageSpec {
        typescript_language_spec()
    }

    fn default_excludes(&self) -> &'static [&'static str] {
        &["node_modules", "dist", "build", ".next", "coverage"]
    }

    fn module_path(&self, _root: &Path, relative_path: &Path) -> ModulePath {
        let components: Vec<String> = relative_path
            .components()
            .filter_map(|c| match c {
                Component::Normal(seg) => seg.to_str().map(String::from),
                _ => None,
            })
            .collect();

        if components.is_empty() {
            return ModulePath::new(vec![String::from("index")]);
        }

        let last_idx = components.len() - 1;
        let mut segments = Vec::with_capacity(components.len());
        for (idx, comp) in components.iter().enumerate() {
            if idx == last_idx {
                let without_ext = comp.strip_suffix(".ts").unwrap_or(comp);
                let stem = without_ext
                    .strip_suffix(".test")
                    .or_else(|| without_ext.strip_suffix(".spec"))
                    .unwrap_or(without_ext);
                if stem != "index" && !stem.is_empty() {
                    segments.push(stem.to_string());
                }
            } else {
                segments.push(comp.clone());
            }
        }

        if segments.is_empty() {
            segments.push(String::from("index"));
        }

        ModulePath::new(segments)
    }

    fn file_role(&self, relative_path: &Path) -> FileRole {
        let role = common::classify_file_role(
            relative_path,
            &["tests", "__tests__", "e2e"],
            &[".test.ts", ".spec.ts"],
            &[],
            &["main.ts", "cli.ts"],
        );
        if role != FileRole::Module {
            return role;
        }
        let normal: Vec<&str> = relative_path
            .components()
            .filter_map(|c| match c {
                Component::Normal(seg) => seg.to_str(),
                _ => None,
            })
            .collect();
        if normal.as_slice() == ["index.ts"] || normal.as_slice() == ["src", "index.ts"] {
            return FileRole::LibraryRoot;
        }
        FileRole::Module
    }

    fn extract_module(
        &self,
        tree: &SyntaxTree,
        module_path: ModulePath,
        role: FileRole,
    ) -> FileAst {
        TypeScriptExtractor::new(tree, module_path).extract_file(role)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::codebase::Codebase;
    use crate::ast::item::{
        CallKind, CallReceiver, FunctionRole, ImportKind, ParamAccess, TypeKind, TypeRef,
        Visibility,
    };
    use crate::config::Config;
    use crate::rules::Rule;
    use crate::rules::cohesion::CohesionRule;
    use crate::rules::test_coupling::TestCouplingRule;

    fn parse_ts(rel_path: &str, src: &str) -> (SyntaxTree, FileAst) {
        let tree = SyntaxTree::parse(rel_path, rel_path, typescript_language_spec(), src);
        let ast = TypeScriptFrontend.extract(&tree, Path::new("."));
        (tree, ast)
    }

    #[test]
    fn infers_typescript_module_paths() {
        let f = TypeScriptFrontend;
        let cases = [
            ("src/order.ts", "src::order"),
            ("src/order.test.ts", "src::order"),
            ("src/order.spec.ts", "src::order"),
            ("src/models/index.ts", "src::models"),
            ("index.ts", "index"),
            ("order.ts", "order"),
        ];
        for (path, expected) in cases {
            assert_eq!(
                f.module_path(Path::new("."), Path::new(path)).to_string(),
                expected
            );
        }
    }

    #[test]
    fn classifies_typescript_file_roles() {
        let f = TypeScriptFrontend;
        let cases = [
            ("tests/e2e.test.ts", FileRole::IntegrationTest),
            ("__tests__/order.spec.ts", FileRole::IntegrationTest),
            ("e2e/flow.ts", FileRole::IntegrationTest),
            ("src/order.test.ts", FileRole::UnitTest),
            ("src/order.spec.ts", FileRole::UnitTest),
            ("index.ts", FileRole::LibraryRoot),
            ("src/index.ts", FileRole::LibraryRoot),
            ("src/models/index.ts", FileRole::Module),
            ("src/main.ts", FileRole::BinaryEntrypoint),
            ("src/cli.ts", FileRole::BinaryEntrypoint),
            ("src/order.ts", FileRole::Module),
        ];
        for (path, expected) in cases {
            assert_eq!(f.file_role(Path::new(path)), expected);
        }
    }

    #[test]
    fn extracts_class_fields_visibility_and_parameter_properties() {
        let src = r"
export default class Order {
    public label: string = '';
    #secret: string = 'x';
    protected auditId: number = 0;

    constructor(private total: number, public id: string, raw: number) {
        this.auditId = raw;
    }
}

class LocalHelper {}
";
        let (_, ast) = parse_ts("src/models/order.ts", src);
        assert_eq!(ast.types.len(), 2);

        let order = &ast.types[0];
        assert_eq!(order.name, "Order");
        assert_eq!(order.kind, TypeKind::Class);
        assert_eq!(order.visibility, Visibility::Public);
        assert!(order.is_default_export);

        let field_summary: Vec<(&str, Visibility)> = order
            .fields
            .iter()
            .map(|f| (f.name.as_str(), f.visibility))
            .collect();
        assert_eq!(
            field_summary,
            vec![
                ("label", Visibility::Public),
                ("#secret", Visibility::Private),
                ("auditId", Visibility::Internal),
                ("total", Visibility::Private),
                ("id", Visibility::Public),
            ]
        );
        assert_eq!(ast.types[1].name, "LocalHelper");
        assert_eq!(ast.types[1].visibility, Visibility::Private);
    }

    #[test]
    fn extracts_constructors_static_methods_and_instance_methods() {
        let src = r"
export class Order {
    constructor(private total: number) {}

    static create(total: number): Order {
        return new Order(total);
    }

    static helper(x: number): number {
        return x * 2;
    }

    private normalize(): void {
        this.total = Math.max(0, this.total);
    }

    #internalReset(): void {
        this.total = 0;
    }

    public applyDiscount(rate: number): number {
        this.normalize();
        this.#internalReset();
        return this.total * (1 - rate);
    }
}
";
        let (_, ast) = parse_ts("src/models/order.ts", src);
        let summary: Vec<(&str, Visibility, bool, bool)> = ast
            .functions
            .iter()
            .map(|f| {
                (
                    f.name.as_str(),
                    f.visibility,
                    f.has_self_receiver(),
                    f.returns_self_type(),
                )
            })
            .collect();
        assert_eq!(
            summary,
            vec![
                ("constructor", Visibility::Public, false, true),
                ("create", Visibility::Public, false, true),
                ("helper", Visibility::Public, false, false),
                ("normalize", Visibility::Private, true, false),
                ("#internalReset", Visibility::Private, true, false),
                ("applyDiscount", Visibility::Public, true, false),
            ]
        );
        assert_eq!(ast.functions[0].return_type, Some(TypeRef::SelfType));
        assert_eq!(
            ast.functions[5].self_receiver,
            Some(ParamAccess::MutableRef)
        );
        assert!(ast.functions[5].calls.iter().any(|c| {
            c.kind == CallKind::Method
                && c.callee == "normalize"
                && c.receiver == Some(CallReceiver::SelfInstance)
        }));
    }

    #[test]
    fn resolves_es_imports_barrel_reexports_and_arrow_functions_end_to_end() {
        let (t1, f1) = parse_ts(
            "src/models/order.ts",
            "export default class Order { public total: number = 0; }\n",
        );
        let (t2, f2) = parse_ts(
            "src/models/index.ts",
            "export { default as CustomerOrder } from './order';\nexport * from './order';\n",
        );
        let (t3, f3) = parse_ts(
            "src/service/billing.ts",
            r"
import DefaultOrder from '../models/order';
import { CustomerOrder as Aliased } from '../models';
import * as models from '../models';

export const compute = (a: DefaultOrder, b: Aliased, c: models.Order): number => {
    a.total += b.total + c.total;
    const x = new models.Order();
    return models.helper(a) + x.total;
};
",
        );

        assert_eq!(f2.module_path.to_string(), "src::models");
        assert_eq!(f2.exports.len(), 2);
        assert_eq!(f2.exports[0].exported_name, "CustomerOrder");
        assert!(f2.exports[1].is_wildcard);

        let import_kinds: Vec<(ImportKind, &str)> = f3
            .imports
            .iter()
            .map(|i| (i.kind, i.local_name()))
            .collect();
        assert_eq!(
            import_kinds,
            vec![
                (ImportKind::Default, "DefaultOrder"),
                (ImportKind::Named, "Aliased"),
                (ImportKind::Namespace, "models"),
            ]
        );

        let codebase = Codebase::build(vec![t1, t2, t3], vec![f1, f2, f3]);
        let order_id = codebase.types()[0].id;
        let compute = &codebase.functions()[0];
        for param in &compute.resolved_params {
            assert_eq!(param.resolved_type, Some(order_id));
        }
        assert!(
            compute
                .decl
                .field_accesses
                .iter()
                .any(|a| a.receiver_param == "a" && a.field_name == "total" && a.is_write)
        );
        assert!(compute.resolved_constructions.contains(&order_id));
        assert!(compute.decl.calls.iter().any(|c| {
            c.kind == CallKind::Scoped
                && c.callee == "helper"
                && c.receiver == Some(CallReceiver::Named("src::models".into()))
        }));
    }

    #[test]
    fn extracts_describe_it_tests_and_runs_polyglot_rules_without_rule_changes() {
        let (t_order, f_order) = parse_ts(
            "src/order.ts",
            r"
export class Order {
    constructor(private secret: number, public total: number) {}
    static create(total: number): Order { return new Order(1, total); }
    static statelessHelper(x: number): number { return x + 1; }
    private internalStep(): void { this.secret += 1; }
    public applyDiscount(rate: number): number {
        this.internalStep();
        return this.total * (1 - rate);
    }
}
",
        );
        let (t_test, f_test) = parse_ts(
            "src/order.test.ts",
            r#"
import { Order } from './order';

function makeFixture(): Order {
    return new Order(1, 100);
}

describe("Order suite", () => {
    describe("nested", () => {
        it("applies discount", () => {
            const o = makeFixture();
            o.applyDiscount(0.1);
            (o as any).internalStep();
        });

        test("creates via factory", () => {
            const created = Order.create(50);
            created.applyDiscount(0.2);
        });
    });
});
"#,
        );

        assert_eq!(f_test.role, FileRole::UnitTest);
        assert_eq!(f_test.module_path, f_order.module_path);
        assert_eq!(f_test.functions[0].role, FunctionRole::TestHelper);
        assert_eq!(f_test.functions[1].name, "applies discount");
        assert_eq!(f_test.functions[1].role, FunctionRole::Test);
        assert_eq!(f_test.functions[2].name, "creates via factory");
        assert_eq!(f_test.functions[2].role, FunctionRole::Test);

        let codebase = Codebase::build(vec![t_order, t_test], vec![f_order, f_test]);
        let cfg = Config::default();

        let cohesion_diags = CohesionRule.check(&codebase, &cfg);
        assert!(
            cohesion_diags
                .iter()
                .any(|d| d.rule_id() == "stateless-impl-method"
                    && d.message().contains("Order.statelessHelper")),
            "expected stateless-impl-method on Order.statelessHelper, got: {cohesion_diags:?}"
        );
        assert!(
            !cohesion_diags
                .iter()
                .any(|d| d.message().contains("Order.constructor")
                    || d.message().contains("Order.create")),
            "constructor and factory returning Order must not be flagged: {cohesion_diags:?}"
        );

        let coupling_diags = TestCouplingRule.check(&codebase, &cfg);
        assert!(
            coupling_diags
                .iter()
                .any(|d| d.rule_id() == "test-calls-private-api"
                    && d.message().contains("Order.internalStep")),
            "expected test-calls-private-api on Order.internalStep, got: {coupling_diags:?}"
        );
    }

    #[test]
    fn normalizes_typescript_tokens_across_renamed_vars_and_distinguishes_loops() {
        let src = r"
function calcA(price: number, tax: number): number {
    const subtotal = price + tax;
    if (subtotal > 100) {
        return subtotal - 10;
    }
    return subtotal;
}

function calcB(cost: number, fee: number): number {
    let total = cost + fee;
    if (total > 250) {
        return total - 25;
    }
    return total;
}

function calcLoop(items: number[]): number {
    let sum = 0;
    for (const item of items) {
        sum += item;
    }
    return sum;
}
";
        let (_, ast) = parse_ts("src/math.ts", src);
        let tokens_a = ast.functions[0].body_tokens.tokens();
        let tokens_b = ast.functions[1].body_tokens.tokens();
        let tokens_loop = ast.functions[2].body_tokens.tokens();

        assert_eq!(tokens_a, tokens_b);
        assert_ne!(tokens_a, tokens_loop);
        assert!(tokens_a.contains(&"@bind".to_string()));
        assert!(tokens_a.contains(&"@if".to_string()));
        assert!(tokens_a.contains(&"@return".to_string()));
        assert!(tokens_loop.contains(&"@loop".to_string()));
    }
}
