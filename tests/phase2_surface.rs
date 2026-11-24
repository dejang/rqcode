use rqcode::ast::codebase::{Codebase, EffectiveVisibility};
use rqcode::ast::{FileAst, SyntaxTree};
use rqcode::config::Config;
use rqcode::diagnostic::Diagnostic;
use rqcode::lang::LanguageFrontend;
use rqcode::lang::rust::{RustFrontend, rust_language_spec};
use rqcode::lang::swift::{SwiftFrontend, swift_language_spec};
use rqcode::lang::typescript::{TypeScriptFrontend, typescript_language_spec};
use rqcode::rules::Rule;
use rqcode::rules::test_coupling::TestCouplingRule;
use std::path::Path;

fn parse_rust(rel_path: &str, src: &str) -> (SyntaxTree, FileAst) {
    let tree = SyntaxTree::parse(rel_path, rel_path, rust_language_spec(), src);
    let ast = RustFrontend.extract(&tree, Path::new("."));
    (tree, ast)
}

fn parse_ts(rel_path: &str, src: &str) -> (SyntaxTree, FileAst) {
    let tree = SyntaxTree::parse(rel_path, rel_path, typescript_language_spec(), src);
    let ast = TypeScriptFrontend.extract(&tree, Path::new("."));
    (tree, ast)
}

fn parse_swift(rel_path: &str, src: &str) -> (SyntaxTree, FileAst) {
    let tree = SyntaxTree::parse(rel_path, rel_path, swift_language_spec(), src);
    let ast = SwiftFrontend::default().extract(&tree, Path::new("."));
    (tree, ast)
}

fn check_coupling(items: Vec<(SyntaxTree, FileAst)>) -> (Codebase, Vec<Diagnostic>) {
    let (trees, files): (Vec<_>, Vec<_>) = items.into_iter().unzip();
    let codebase = Codebase::build(trees, files);
    let diags = TestCouplingRule.check(&codebase, &Config::default());
    (codebase, diags)
}

#[test]
fn mixed_rust_and_typescript_codebase_keeps_surfaces_per_language() {
    let (_cb, diags) = check_coupling(vec![
        parse_rust("src/lib.rs", "pub mod api;"),
        parse_rust("src/api.rs", "pub fn ping() -> u32 { 1 }"),
        parse_ts(
            "web/util.ts",
            "export function formatPrice(n: number): string { return `${n}`; }",
        ),
        parse_ts(
            "web/util.test.ts",
            "import { formatPrice } from './util';\ntest('formats', () => { formatPrice(10); });",
        ),
    ]);
    assert!(
        diags.is_empty(),
        "TS files without a barrel must not be marked internal by a Rust lib.rs: {diags:#?}"
    );
}

#[test]
fn nested_barrel_outermost_boundary_wins() {
    let (_cb, diags) = check_coupling(vec![
        parse_ts(
            "src/pkg/index.ts",
            "export { PublicApi } from './sub/impl';",
        ),
        parse_ts(
            "src/pkg/sub/index.ts",
            "export { OtherApi } from './other';",
        ),
        parse_ts("src/pkg/sub/other.ts", "export class OtherApi { run() {} }"),
        parse_ts(
            "src/pkg/sub/impl.ts",
            "export class PublicApi { execute(): number { return 1; } }",
        ),
        parse_ts(
            "tests/pkg.test.ts",
            "import { PublicApi } from '../src/pkg';\ntest('uses outer barrel', () => { new PublicApi().execute(); });",
        ),
    ]);
    assert!(
        diags.is_empty(),
        "Symbol exported by outermost barrel should remain public even if inner barrel omits it: {diags:#?}"
    );
}

#[test]
fn type_returned_from_public_factory_is_effectively_public() {
    let (cb, diags) = check_coupling(vec![
        parse_ts(
            "src/pkg/index.ts",
            "export { createClient } from './factory';",
        ),
        parse_ts(
            "src/pkg/client.ts",
            "export class Client { fetch(): number { return 42; } }",
        ),
        parse_ts(
            "src/pkg/factory.ts",
            "import { Client } from './client';\nexport function createClient(): Client { return new Client(); }",
        ),
        parse_ts(
            "tests/factory.test.ts",
            "import { createClient } from '../src/pkg';\ntest('works', () => { const c = createClient(); c.fetch(); });",
        ),
    ]);
    let vis = cb
        .types()
        .iter()
        .find(|t| t.decl.name == "Client")
        .map(|ty| cb.public_surface().effective_type_visibility(ty));
    assert_eq!(vis, Some(EffectiveVisibility::Public));
    assert!(diags.is_empty(), "Unexpected diagnostics: {diags:#?}");
}

#[test]
fn rust_pub_use_module_alias_and_pub_self_visibility() {
    let (cb, diags) = check_coupling(vec![
        parse_rust("src/lib.rs", "mod inner;\npub use inner as api;"),
        parse_rust(
            "src/inner.rs",
            "pub fn run() -> u32 { helper() }\npub(self) fn helper() -> u32 { 7 }",
        ),
        parse_rust(
            "tests/integration.rs",
            "use crate::api::run;\n#[test]\nfn calls_aliased_module() { assert_eq!(run(), 7); }",
        ),
    ]);
    let vis_of = |name: &str| {
        cb.functions()
            .iter()
            .find(|f| f.decl.name == name)
            .map(|f| cb.public_surface().effective_func_visibility(f, cb.types()))
    };
    assert_eq!(vis_of("run"), Some(EffectiveVisibility::Public));
    assert_eq!(vis_of("helper"), Some(EffectiveVisibility::Private));
    assert!(diags.is_empty(), "Unexpected diagnostics: {diags:#?}");
}

#[test]
fn ts_wildcard_does_not_expose_default_function_and_namespace_two_step_reexports_work() {
    let (_cb, diags) = check_coupling(vec![
        parse_ts(
            "src/mod/index.ts",
            "export * from './def';\nexport * as ns from './ns_mod';\nimport { TwoStep } from './two';\nexport { TwoStep };",
        ),
        parse_ts(
            "src/mod/def.ts",
            "export default function hiddenDefault(): number { return 1; }\nexport function useDef(): number { return hiddenDefault(); }",
        ),
        parse_ts(
            "src/mod/ns_mod.ts",
            "export function nsFn(): number { return 2; }",
        ),
        parse_ts(
            "src/mod/two.ts",
            "export class TwoStep { go(): number { return 3; } }",
        ),
        parse_ts(
            "tests/mod.test.ts",
            "import hiddenDefault from '../src/mod/def';\nimport { ns, TwoStep } from '../src/mod';\ntest('bypass default', () => { hiddenDefault(); ns.nsFn(); new TwoStep().go(); });",
        ),
    ]);
    let bypasses: Vec<_> = diags
        .iter()
        .filter(|d| d.rule_id() == "test-bypasses-public-surface")
        .collect();
    assert_eq!(
        bypasses.len(),
        1,
        "Expected only default function bypass, got: {diags:#?}"
    );
    assert!(bypasses[0].message().contains("hiddenDefault"));
}

#[test]
fn non_reexporting_ts_index_file_is_not_treated_as_barrel() {
    let (_cb, diags) = check_coupling(vec![
        parse_ts(
            "src/pages/index.ts",
            "export default function Home(): number { return 1; }",
        ),
        parse_ts(
            "src/pages/helper.ts",
            "export function pageHelper(): number { return 2; }",
        ),
        parse_ts(
            "tests/page.test.ts",
            "import { pageHelper } from '../src/pages/helper';\ntest('uses helper', () => { pageHelper(); });",
        ),
    ]);
    assert!(
        diags.is_empty(),
        "Non-reexporting index.ts must not act as a barrel: {diags:#?}"
    );
}

#[test]
fn rust_hybrid_lib_and_main_isolates_binary_only_modules() {
    let (_cb, diags) = check_coupling(vec![
        parse_rust("src/lib.rs", "pub mod core;"),
        parse_rust("src/core.rs", "pub fn work() -> u32 { 1 }"),
        parse_rust("src/main.rs", "mod cli;\nfn main() { cli::run(); }"),
        parse_rust(
            "src/cli.rs",
            "pub fn run() { let _ = crate::core::work(); }",
        ),
        parse_rust(
            "tests/cli_test.rs",
            "use crate::cli::run;\n#[test]\nfn runs_cli() { run(); }",
        ),
    ]);
    assert!(
        diags.is_empty(),
        "Binary-only module in hybrid crate must not trigger library surface bypass: {diags:#?}"
    );
}

#[test]
fn private_trait_impl_on_public_type_is_not_effectively_public() {
    let (cb, _diags) = check_coupling(vec![
        parse_rust("src/lib.rs", "pub mod item;"),
        parse_rust(
            "src/item.rs",
            "pub struct Item;\ntrait SecretTrait { fn secret(&self) -> u32; }\nimpl SecretTrait for Item { fn secret(&self) -> u32 { 9 } }",
        ),
    ]);
    let vis = cb
        .functions()
        .iter()
        .find(|f| f.decl.name == "secret")
        .map(|f| cb.public_surface().effective_func_visibility(f, cb.types()));
    assert_eq!(vis, Some(EffectiveVisibility::Private));
}

#[test]
fn overloaded_swift_initializers_do_not_falsely_trigger_overexposed() {
    let (_cb, diags) = check_coupling(vec![
        parse_swift(
            "Sources/Kit/Client.swift",
            "public struct Client {\n    public init() {}\n    internal init(seed: Int) {}\n    public static func make() -> Client { Client(seed: 1) }\n}",
        ),
        parse_swift(
            "Tests/KitTests/ClientTests.swift",
            "import Kit\nfinal class ClientTests: XCTestCase {\n    func testPublicInit() { _ = Client() }\n}",
        ),
    ]);
    let overexposed: Vec<_> = diags
        .iter()
        .filter(|d| d.rule_id() == "overexposed-for-tests")
        .collect();
    assert!(
        overexposed.is_empty(),
        "Ambiguous overloaded init must not trigger overexposed-for-tests: {diags:#?}"
    );
}

#[test]
fn host_module_production_collaborator_is_exempt_from_surface_bypass() {
    let (_cb, diags) = check_coupling(vec![
        parse_rust("src/lib.rs", "mod lexer;\npub mod parser;"),
        parse_rust(
            "src/lexer.rs",
            "pub struct Lexer;\nimpl Lexer { pub fn new() -> Self { Self } }",
        ),
        parse_rust(
            "src/parser.rs",
            "use crate::lexer::Lexer;\npub fn parse() -> Lexer { Lexer::new() }\n#[cfg(test)]\nmod tests {\n    use super::parse;\n    use crate::lexer::Lexer;\n    #[test]\n    fn checks_parser() { let _ = Lexer::new(); let _ = parse(); }\n}",
        ),
    ]);
    let bypasses: Vec<_> = diags
        .iter()
        .filter(|d| d.rule_id() == "test-bypasses-public-surface")
        .collect();
    assert!(
        bypasses.is_empty(),
        "Host module inline test reusing a collaborator already called by its prod code should be exempt: {diags:#?}"
    );
}
