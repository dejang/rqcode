use rqcode::ast::codebase::Codebase;
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

fn check_coupling(items: Vec<(SyntaxTree, FileAst)>) -> Vec<Diagnostic> {
    let (trees, files): (Vec<_>, Vec<_>) = items.into_iter().unzip();
    let codebase = Codebase::build(trees, files);
    TestCouplingRule.check(&codebase, &Config::default())
}

fn assert_rule_messages(diags: &[Diagnostic], rule_id: &str, expected: &[&str]) {
    let matching: Vec<&Diagnostic> = diags.iter().filter(|d| d.rule_id() == rule_id).collect();
    assert_eq!(
        matching.len(),
        expected.len(),
        "expected {} `{rule_id}` diagnostic(s), got: {matching:#?}",
        expected.len()
    );
    for needle in expected {
        assert!(
            matching.iter().any(|d| d.message().contains(needle)),
            "expected `{rule_id}` diagnostic containing `{needle}`, got: {matching:#?}"
        );
    }
}

#[test]
fn interaction_only_test_exempts_callback_di_and_counts_helper_and_expect_soft_assertions() {
    let diags = check_coupling(vec![
        parse_ts(
            "src/pricing.ts",
            "export function compute(x: number): number { return x * 2; }",
        ),
        parse_ts(
            "src/button.ts",
            r#"
import { compute } from "./pricing";
export class Button {
  public click(onClick: (n: number) => void): number {
    const out = compute(5);
    onClick(out);
    return out;
  }
}
"#,
        ),
        parse_ts(
            "src/button.test.ts",
            r#"
import { vi, test, expect } from "vitest";
import { Button } from "./button";
test("callback DI interaction is exempt", () => {
  const onClick = vi.fn();
  const btn = new Button();
  btn.click(onClick);
  expect(onClick).toHaveBeenCalledOnce();
});
"#,
        ),
        parse_ts(
            "src/coupled.test.ts",
            r#"
import { vi, test, expect } from "vitest";
import { Button } from "./button";
import { compute } from "./pricing";
vi.mock("./pricing", () => ({ compute: vi.fn(() => 10) }));
function assertPositive(n: number) { expect(n).toBe(10); }
test("helper behavioral assertion exempts test", () => {
  const btn = new Button();
  const out = btn.click(() => {});
  assertPositive(out);
  expect(compute).toHaveBeenCalledOnce();
});
test("expect.soft behavioral assertion exempts test", () => {
  const btn = new Button();
  const out = btn.click(() => {});
  expect.soft(out).toBe(10);
  expect(compute).toHaveBeenCalledOnce();
});
test("internal mock with only toHaveBeenCalledOnce is flagged", () => {
  const btn = new Button();
  btn.click(() => {});
  expect(compute).toHaveBeenCalledOnce();
});
"#,
        ),
    ]);

    assert_rule_messages(
        &diags,
        "interaction-only-test",
        &["internal mock with only toHaveBeenCalledOnce is flagged"],
    );
}

#[test]
fn spy_on_sut_exempts_collaborator_fault_injection_and_external_boundary_while_catching_as_any_and_prototype()
 {
    let diags = check_coupling(vec![
        parse_ts(
            "src/repo.ts",
            r"
export class MemoryRepo {
  private items: number[] = [];
  public insert(n: number): void { this.items.push(n); }
  public save(n: number): void { this.items.push(n); }
}
",
        ),
        parse_ts(
            "src/http.ts",
            r"
export class HttpClient {
  public async get(url: string): Promise<string> {
    const res = await fetch(url);
    return this.decode(res);
  }
  public decode(res: unknown): string { return String(res); }
}
",
        ),
        parse_ts(
            "src/workflow.ts",
            r#"
import { MemoryRepo } from "./repo";
export class Workflow {
  public run(repo: MemoryRepo): number {
    this.validate();
    this.secretStep();
    repo.save(1);
    return 1;
  }
  public validate(): void {}
  private secretStep(): void {}
}
"#,
        ),
        parse_ts(
            "src/workflow.test.ts",
            r#"
import { vi, test, expect } from "vitest";
import { MemoryRepo } from "./repo";
import { HttpClient } from "./http";
import { Workflow } from "./workflow";
test("collaborator fault injection is exempt", () => {
  const repo = new MemoryRepo();
  repo.insert(1);
  vi.spyOn(repo, "save");
  const wf = new Workflow();
  expect(wf.run(repo)).toBe(1);
});
test("external boundary ambient fetch spy is exempt", async () => {
  const http = new HttpClient();
  vi.spyOn(http, "decode");
  await http.get("https://example.com");
  expect(http.decode).toHaveBeenCalledTimes(1);
});
test("as any spy on private SUT method is flagged", () => {
  const repo = new MemoryRepo();
  const wf = new Workflow();
  const spy = vi.spyOn(wf as any, "secretStep");
  expect(wf.run(repo)).toBe(1);
  expect(spy).toHaveBeenCalledTimes(1);
});
test("prototype spy on reachable SUT method is flagged", () => {
  const repo = new MemoryRepo();
  const wf = new Workflow();
  const spy = vi.spyOn(Workflow.prototype, "validate");
  expect(wf.run(repo)).toBe(1);
  expect(spy).toHaveBeenCalledTimes(1);
});
"#,
        ),
    ]);

    assert_rule_messages(&diags, "test-spies-on-sut", &["secretStep", "validate"]);
}

#[test]
fn module_mock_supports_do_mock_import_expression_and_barrel_external_boundary_propagation() {
    let diags = check_coupling(vec![
        parse_ts(
            "src/pricing.ts",
            "export function price(x: number): number { return x * 2; }",
        ),
        parse_ts(
            "src/tax.ts",
            "export function tax(x: number): number { return x * 0.1; }",
        ),
        parse_ts(
            "src/gateway/http.ts",
            "export async function fetchUser(id: string): Promise<unknown> { return fetch(id); }",
        ),
        parse_ts(
            "src/gateway/index.ts",
            "export { fetchUser } from './http';",
        ),
        parse_ts(
            "src/order.test.ts",
            r#"
import { vi, test, expect } from "vitest";
vi.mock(import("./tax"), () => ({ tax: vi.fn(() => 0) }));
vi.mock("./gateway", () => ({ fetchUser: vi.fn(async () => ({})) }));
vi.mock("@/internal/unmapped", () => ({ helper: vi.fn() }));
test("in-body vi.doMock is detected", () => {
  vi.doMock("./pricing", () => ({ price: vi.fn(() => 42) }));
  expect(42).toBe(42);
});
"#,
        ),
    ]);

    assert_rule_messages(
        &diags,
        "test-mocks-internal-collaborator",
        &["./tax", "./pricing"],
    );
}

#[test]
fn mock_only_abstraction_counts_distinct_stdlib_impls_transitively_and_handles_sealed_vs_binary_subtraits()
 {
    // 1. Rust library: 2 distinct stdlib impls + blanket generic + sealed trait -> 0 diagnostics.
    let lib_diags = check_coupling(vec![parse_rust(
        "src/lib.rs",
        r"
mod sealed { pub trait Sealed {} }
pub trait SealedApi: sealed::Sealed { fn id(&self) -> u32; }
pub struct RealSealed;
impl sealed::Sealed for RealSealed {}
impl SealedApi for RealSealed { fn id(&self) -> u32 { 1 } }

pub(crate) trait Codec { fn encode(&self) -> usize; }
impl Codec for String { fn encode(&self) -> usize { self.len() } }
impl Codec for std::path::PathBuf { fn encode(&self) -> usize { 1 } }

pub(crate) trait Handler { fn call(&self); }
impl<F: Fn()> Handler for F { fn call(&self) { self(); } }

#[cfg(test)]
mod tests {
    use super::*;
    struct FakeCodec;
    impl Codec for FakeCodec { fn encode(&self) -> usize { 0 } }
    struct FakeHandler;
    impl Handler for FakeHandler { fn call(&self) {} }
    struct FakeSealed;
    impl sealed::Sealed for FakeSealed {}
    impl SealedApi for FakeSealed { fn id(&self) -> u32 { 0 } }
}
",
    )]);
    assert_rule_messages(&lib_diags, "mock-only-abstraction", &[]);

    // 2. Binary-only Rust crate: non-sealed subtrait + inline `impl super::Sub for Fake` -> FLAGGED.
    let bin_diags = check_coupling(vec![parse_rust(
        "src/main.rs",
        r"
pub trait Base { fn base(&self) -> u32; }
pub trait Sub: Base { fn work(&self) -> u32; }
pub struct LiveWorker;
impl Base for LiveWorker { fn base(&self) -> u32 { 1 } }
impl Sub for LiveWorker { fn work(&self) -> u32 { self.base() } }
fn main() { let _ = LiveWorker.work(); }
#[cfg(test)]
mod tests {
    struct FakeWorker;
    impl super::Base for FakeWorker { fn base(&self) -> u32 { 0 } }
    impl super::Sub for FakeWorker { fn work(&self) -> u32 { 0 } }
}
",
    )]);
    assert_rule_messages(&bin_diags, "mock-only-abstraction", &["`Base`", "`Sub`"]);

    // 3. TS transitive subclassing: Repo (BaseRepo + CachedRepo) exempt; SoloRepo (OneRepo) flagged.
    let ts_diags = check_coupling(vec![
        parse_ts("src/index.ts", "export { run } from './repo';"),
        parse_ts(
            "src/repo.ts",
            r"
export interface Repo { get(): number; }
class BaseRepo implements Repo { public get(): number { return 1; } }
class CachedRepo extends BaseRepo { public override get(): number { return 2; } }
export interface SoloRepo { load(): number; }
class OneRepo implements SoloRepo { public load(): number { return 1; } }
export function run(): number { return new CachedRepo().get() + new OneRepo().load(); }
",
        ),
        parse_ts(
            "src/repo.test.ts",
            r#"
import { run, type Repo, type SoloRepo } from "./repo";
class MockRepo implements Repo { public get(): number { return 0; } }
class MockSolo implements SoloRepo { public load(): number { return 0; } }
test("runs", () => { run(); new MockRepo().get(); new MockSolo().load(); });
"#,
        ),
    ]);
    assert_rule_messages(&ts_diags, "mock-only-abstraction", &["`SoloRepo`"]);
}

#[test]
fn rust_debug_repr_ignores_assert_ne_custom_msg_chained_calls_and_compound_exprs_while_catching_rhs_and_positional()
 {
    let diags = check_coupling(vec![parse_rust(
        "src/lib.rs",
        r#"
#[derive(Debug)]
pub struct Token { pub public_id: u32, secret: u32 }
impl Token {
    pub fn new(id: u32) -> Self { Self { public_id: id, secret: id * 7 } }
    pub fn make_tok() -> Self { Self::new(1) }
}
#[cfg(test)]
mod tests {
    use super::Token;
    #[test]
    fn negative_controls_do_not_trigger() {
        let tok = Token::new(1);
        let other = Token::new(2);
        let num = 42u32;
        assert_ne!(format!("{:?}", tok), "Token { public_id: 2, secret: 14 }");
        assert_eq!(format!("{:?}", tok), format!("{:?}", other));
        assert_eq!(tok.public_id, 1, "unexpected token: {:?}", tok);
        assert_eq!(format!("{:?}", tok).contains("public_id"), true);
        assert_eq!(format!("{} {:?}", tok, num), "x 42");
        assert_eq!(format!("{:?}", tok.public_id), "1");
        assert_eq!(format!("{:?}", Token::make_tok()), "x");
    }
    #[test]
    fn positive_rhs_and_positional_trigger() {
        let tok = Token::new(1);
        let num = 42u32;
        assert_eq!("Token { public_id: 1, secret: 7 }", format!("{:?}", tok));
        assert_eq!(format!("{} {:?}", num, tok), "42 Token { public_id: 1, secret: 7 }");
        assert_eq!(format!("{0:?}", tok), "Token { public_id: 1, secret: 7 }");
    }
}
"#,
    )]);

    assert_rule_messages(
        &diags,
        "test-asserts-internal-representation",
        &[
            "positive_rhs_and_positional_trigger",
            "positive_rhs_and_positional_trigger",
            "positive_rhs_and_positional_trigger",
        ],
    );
    assert!(
        diags
            .iter()
            .filter(|d| d.rule_id() == "test-asserts-internal-representation")
            .all(|d| d.message().contains("positive_rhs_and_positional_trigger")),
        "negative control test must not emit repr diagnostics: {diags:#?}"
    );
}

#[test]
fn rust_cfg_test_item_impls_and_ambient_io_boundaries_work_accurately() {
    let diags = check_coupling(vec![
        parse_rust(
            "src/store.rs",
            r#"
pub(crate) trait Store { fn load(&self) -> String; }
pub(crate) struct FileStore;
impl Store for FileStore {
    fn load(&self) -> String { std::fs::read_to_string("data.txt").unwrap_or_default() }
}
#[cfg(test)]
struct FakeStore;
#[cfg(test)]
impl Store for FakeStore { fn load(&self) -> String { String::new() } }
"#,
        ),
        parse_rust(
            "src/calc.rs",
            r"
use std::time::Duration;
use std::net::IpAddr;
pub(crate) trait Calc { fn timeout(&self, ip: IpAddr) -> Duration; }
pub(crate) struct LiveCalc;
impl Calc for LiveCalc { fn timeout(&self, _ip: IpAddr) -> Duration { Duration::from_secs(1) } }
#[cfg(test)]
struct FakeCalc;
#[cfg(test)]
impl Calc for FakeCalc { fn timeout(&self, _ip: IpAddr) -> Duration { Duration::from_secs(0) } }
",
        ),
        parse_rust("src/lib.rs", "mod store;\nmod calc;\n"),
    ]);

    assert_rule_messages(&diags, "mock-only-abstraction", &["`Calc`"]);
}

#[test]
fn swift_inheritance_extensions_debug_guards_and_mirror_escapes_work_end_to_end() {
    let diags = check_coupling(vec![
        parse_swift(
            "Sources/App/Pricing.swift",
            r"
protocol PricingEngine { func quote(_ amount: Int) -> Int }
final class LivePricing {
    private var factor: Int = 2
    public func run(_ x: Int) -> Int { return quote(x) }
}
extension LivePricing: PricingEngine {
    func quote(_ amount: Int) -> Int { return amount * factor }
}
#if !DEBUG
#else
extension LivePricing {
    func debugOnlyReset() { factor = 1 }
}
#endif
#if DEBUG_MODE
extension LivePricing {
    public func betaFeature() -> Int { return factor }
}
#endif
",
        ),
        parse_swift(
            "Tests/AppTests/PricingTests.swift",
            r"
import XCTest
@testable import App
final class MockPricing: PricingEngine {
    func quote(_ amount: Int) -> Int { return amount }
}
final class PricingTests: XCTestCase {
    func testExtensionAndGuardsAndMirror() {
        let sut: LivePricing? = LivePricing()
        sut!.debugOnlyReset()
        _ = sut!.betaFeature()
        let _ = Swift.Mirror(reflecting: sut!)
        let _ = Mirror(reflecting: unknownExternalValue)
        let _ = MockPricing().quote(10)
    }
}
",
        ),
    ]);

    assert_rule_messages(&diags, "mock-only-abstraction", &["PricingEngine"]);
    assert_rule_messages(&diags, "production-test-hook", &["debugOnlyReset"]);
    assert_rule_messages(&diags, "test-encapsulation-escape", &["LivePricing"]);
}

#[test]
fn concrete_base_class_with_production_subclass_is_exempt_from_encapsulation_escape() {
    let diags = check_coupling(vec![
        parse_ts(
            "src/handlers.ts",
            r"
export class BaseHandler { public handle(): number { return 1; } }
export class JsonHandler extends BaseHandler { public override handle(): number { return 2; } }
export class ClosedService { public run(): number { return 3; } }
",
        ),
        parse_ts(
            "src/handlers.test.ts",
            r#"
import { BaseHandler, ClosedService } from "./handlers";
class TestHandler extends BaseHandler {}
class SpyService extends ClosedService {}
test("subclasses", () => { new TestHandler().handle(); new SpyService().run(); });
"#,
        ),
    ]);

    assert_rule_messages(&diags, "test-encapsulation-escape", &["ClosedService"]);
}
