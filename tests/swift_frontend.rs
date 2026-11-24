use rqcode::ast::codebase::Codebase;
use rqcode::ast::item::{
    CallKind, CallReceiver, FunctionDecl, FunctionRole, ImportKind, ParamAccess, TypeKind, TypeRef,
    Visibility,
};
use rqcode::ast::{FileAst, FileRole, SyntaxTree};
use rqcode::config::Config;
use rqcode::lang::swift::{SwiftFrontend, swift_language_spec};
use rqcode::lang::{LanguageFrontend, LanguageRegistry};
use rqcode::rules::RuleRegistry;
use std::path::Path;

fn extract_swift(rel_path: &str, src: &str) -> (SyntaxTree, FileAst) {
    let tree = SyntaxTree::parse(rel_path, rel_path, swift_language_spec(), src);
    let ast = SwiftFrontend::new().extract(&tree, Path::new("."));
    (tree, ast)
}

fn find_fn<'a>(ast: &'a FileAst, name: &str) -> &'a FunctionDecl {
    ast.functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("missing function `{name}`"))
}

fn count_access(func: &FunctionDecl, receiver: &str, field: &str, write: Option<bool>) -> usize {
    func.field_accesses
        .iter()
        .filter(|a| {
            a.receiver_param == receiver
                && a.field_name == field
                && write.is_none_or(|w| a.is_write == w)
        })
        .count()
}

#[test]
fn swiftpm_and_xcode_layouts_map_modules_and_roles() {
    let frontend = SwiftFrontend::new();
    let expected_layout = [
        ("Sources/Shop/Order.swift", "Shop::Order", FileRole::Module),
        ("Sources/Shop/Shop.swift", "Shop", FileRole::Module),
        (
            "Sources/Shop/ProductSpec.swift",
            "Shop::ProductSpec",
            FileRole::Module,
        ),
        (
            "Sources/Shop/LoadTest.swift",
            "Shop::LoadTest",
            FileRole::Module,
        ),
        (
            "MyApp/Networking/SpeedTest.swift",
            "MyApp::Networking::SpeedTest",
            FileRole::Module,
        ),
        (
            "MyApp/Models/OrderSpec.swift",
            "MyApp::Models::OrderSpec",
            FileRole::Module,
        ),
        (
            "Sources/Shop/tests/Fixture.swift",
            "Shop::tests::Fixture",
            FileRole::Module,
        ),
        (
            "Sources/ShopCLI/main.swift",
            "ShopCLI::main",
            FileRole::BinaryEntrypoint,
        ),
        (
            "Packages/Shop/Sources/Shop/Order.swift",
            "Shop::Order",
            FileRole::Module,
        ),
        (
            "Packages/Shop/Sources/Shop/LoadTest.swift",
            "Shop::LoadTest",
            FileRole::Module,
        ),
        (
            "Packages/Shop/Sources/ShopCLI/main.swift",
            "ShopCLI::main",
            FileRole::BinaryEntrypoint,
        ),
        (
            "Packages/Shop/Tests/ShopTests/OrderTests.swift",
            "Shop::Order",
            FileRole::UnitTest,
        ),
        (
            "Tests/ShopTests/OrderTests.swift",
            "Shop::Order",
            FileRole::UnitTest,
        ),
        (
            "Tests/ShopTests/ShopTests.swift",
            "Shop",
            FileRole::UnitTest,
        ),
        (
            "MyAppTests/OrderTests.swift",
            "MyApp::Order",
            FileRole::UnitTest,
        ),
        (
            "MyAppUITests/CheckoutUITests.swift",
            "MyAppUITests::CheckoutUI",
            FileRole::IntegrationTest,
        ),
        (
            "Tests/IntegrationTests/FlowTests.swift",
            "Tests::IntegrationTests::Flow",
            FileRole::IntegrationTest,
        ),
        (
            "Tests/EndToEndTests.swift",
            "Tests::EndToEnd",
            FileRole::IntegrationTest,
        ),
    ];

    for (rel, expected_mod, expected_role) in expected_layout {
        let path = Path::new(rel);
        assert_eq!(
            (
                frontend.module_path(Path::new("."), path).to_string(),
                frontend.file_role(path),
            ),
            (expected_mod.to_string(), expected_role),
            "layout mismatch for {rel}"
        );
    }
}

#[test]
fn extracts_nominal_types_nested_types_protocols_and_imports() {
    let src = r"
@_exported import CoreKit
import struct Foundation.URL

@MainActor
public final class Order {
    public let id: String
    public private(set) var total: Double = 0
    private var secret: Int
    fileprivate var audit: String
    var x = 0, y = 1
    static let maxItems = 100

    public enum Kind { case standard }
    public var label: String { id }
}

extension Order.Kind {
    public func code() -> Int { 1 }
}

extension Array<Order> {
    public func countOrders() -> Int { count }
}

public protocol Payable {
    var amount: Double { get }
}

extension Payable {
    public func withTax() -> Double { amount * 1.1 }
}

actor Ledger {
    private var balance: Double = 0
}
";
    let (_, ast) = extract_swift("Sources/Shop/Order.swift", src);
    assert_eq!(ast.imports.len(), 2);
    assert_eq!(ast.imports[0].kind, ImportKind::Wildcard);
    assert_eq!(ast.imports[1].kind, ImportKind::Named);
    assert_eq!(ast.exports.len(), 1);
    assert!(ast.exports[0].is_wildcard);

    let order = ast
        .types
        .iter()
        .find(|t| t.name == "Order")
        .unwrap_or_else(|| panic!("missing type `Order`"));
    assert!(order.has_annotation("MainActor"));
    let field_names: Vec<&str> = order.fields.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        field_names,
        vec!["id", "total", "secret", "audit", "x", "y"]
    );
    assert_eq!(order.fields[1].visibility, Visibility::Public);
    assert_eq!(order.fields[2].visibility, Visibility::Private);
    assert_eq!(order.fields[3].visibility, Visibility::Private);

    assert!(ast.types.iter().any(|t| t.name == "Order::Kind"));
    assert_eq!(
        find_fn(&ast, "code").owner_type.as_deref(),
        Some("Order::Kind")
    );
    assert_eq!(
        find_fn(&ast, "countOrders").owner_type.as_deref(),
        Some("Array")
    );
    assert!(
        ast.types
            .iter()
            .any(|t| t.name == "Payable" && t.kind == TypeKind::Interface)
    );
    assert_eq!(
        count_access(find_fn(&ast, "withTax"), "self", "amount", Some(false)),
        1
    );
    assert!(
        ast.types
            .iter()
            .any(|t| t.name == "Ledger" && t.kind == TypeKind::Class)
    );
}

#[test]
fn extracts_property_observers_and_subscript_declarations() {
    let src = r"
public struct Ledger {
    public var total: Double = /* initial */ defaultTotal {
        willSet { notifyChange(newValue) }
        didSet { auditUpdate(oldValue) }
    }

    public subscript(index: Int) -> Double {
        get { total + Double(index) }
        set { total = newValue }
    }
}
";
    let (_, ast) = extract_swift("Sources/Shop/Ledger.swift", src);
    let ledger = ast
        .types
        .iter()
        .find(|t| t.name == "Ledger")
        .unwrap_or_else(|| panic!("missing type `Ledger`"));
    assert_eq!(ledger.fields.len(), 1);
    assert_eq!(ledger.fields[0].name, "total");
    assert!(ast.top_level_idents.iter().any(|id| id == "defaultTotal"));

    let observer = find_fn(&ast, "total");
    assert_eq!(observer.owner_type.as_deref(), Some("Ledger"));
    assert_eq!(observer.self_receiver, Some(ParamAccess::MutableRef));
    assert!(observer.calls.iter().any(|c| c.callee == "notifyChange"));
    assert!(observer.calls.iter().any(|c| c.callee == "auditUpdate"));

    let sub = find_fn(&ast, "subscript");
    assert_eq!(sub.owner_type.as_deref(), Some("Ledger"));
    assert_eq!(sub.self_receiver, Some(ParamAccess::MutableRef));
    assert_eq!(sub.params.len(), 1);
    assert_eq!(sub.return_type, Some(TypeRef::Named("Double".into())));
    assert_eq!(count_access(sub, "self", "total", Some(true)), 1);
}

#[test]
fn extracts_implicit_self_subscripts_trailing_labels_and_deep_type_calls() {
    let src = r#"
public struct Order {
    private var total: Double
    private var bonus: Double = 0
    public var customer: Customer
    private var pending: Bool = false
    private var items: [Int] = []
    private var label: String = ""
    var onTap: () -> Void = {}

    private mutating func clamp() {
        total = max(0, total)
        customer.name = ""
        (total, bonus) = (bonus, total)
        self.items[0] = 1
        items[1] = 2
        mutate(&items[0])
        onTap()
    }

    public mutating func applyDiscount(_ rate: Double) -> Double {
        clamp()
        return self.total * (1 - rate)
    }

    public func readWithLabelsAndTrailingClosure() -> (total: Int, pending: Int) {
        let state: State = .pending
        _ = state
        Panel { save() } label: { Caption("Hi") }
        struct LocalRow {
            var total: Int = 0
            func leak(_ o: Order) -> Double { o.bonus + Double(total) }
        }
        _ = A.B.C()
        return (total: 1, pending: 2)
    }
}

extension Order: CustomStringConvertible {
    public var description: String { return String(total) }
}
"#;
    let (_, ast) = extract_swift("Sources/Shop/Order.swift", src);
    let clamp = find_fn(&ast, "clamp");
    assert_eq!(count_access(clamp, "self", "total", Some(true)), 2);
    assert_eq!(count_access(clamp, "self", "bonus", Some(true)), 1);
    assert_eq!(count_access(clamp, "self", "customer", Some(true)), 1);
    assert_eq!(count_access(clamp, "customer", "name", None), 0);
    assert_eq!(count_access(clamp, "self", "items", Some(true)), 3);
    assert!(!clamp.calls.iter().any(|c| c.callee == "items"));
    assert!(clamp.calls.iter().any(|c| {
        c.kind == CallKind::Method
            && c.callee == "onTap"
            && c.receiver == Some(CallReceiver::SelfInstance)
    }));

    let labels_fn = find_fn(&ast, "readWithLabelsAndTrailingClosure");
    assert!(
        labels_fn.field_accesses.is_empty(),
        "tuple labels, trailing closure `label:`, `.pending`, and local struct must not record self reads: {:?}",
        labels_fn.field_accesses
    );
    assert!(
        !labels_fn.local_bindings.iter().any(|b| b.name == "total"),
        "local struct property `total` must not leak into enclosing function local_bindings: {:?}",
        labels_fn.local_bindings
    );
    assert!(
        labels_fn
            .constructed_types
            .contains(&TypeRef::Named("A::B::C".into()))
    );

    let apply = find_fn(&ast, "applyDiscount");
    assert_eq!(apply.self_receiver, Some(ParamAccess::MutableRef));
    assert!(apply.calls.iter().any(|c| {
        c.kind == CallKind::Method
            && c.callee == "clamp"
            && c.receiver == Some(CallReceiver::SelfInstance)
    }));
    assert!(find_fn(&ast, "description").is_interface_impl);
}

#[test]
fn extracts_free_function_inout_writes_comments_in_wrappers_and_subscripts() {
    let src = r#"
public func mutateOrder(_ order: inout Order, rate: Double, i: Int) {
    order.total -= order.total * rate
    (/* keep */ order).total += 1
    adjust(&order.total)
    order.customer.name = ""
    _ = order.items[i]
    let items = [Order]()
    _ = items
}
"#;
    let (_, ast) = extract_swift("Sources/Shop/OrderOps.swift", src);
    let free_fn = find_fn(&ast, "mutateOrder");
    assert_eq!(free_fn.params[0].access, ParamAccess::MutableRef);
    assert_eq!(
        free_fn.params[0].type_name,
        Some(TypeRef::Named("Order".into()))
    );
    assert_eq!(count_access(free_fn, "order", "total", Some(true)), 3);
    assert_eq!(count_access(free_fn, "order", "customer", Some(true)), 1);
    assert_eq!(count_access(free_fn, "order", "items", Some(false)), 1);
    assert!(!free_fn.calls.iter().any(|c| c.callee == "items"));
    assert_eq!(count_access(free_fn, "order", "name", None), 0);
    assert!(
        free_fn
            .constructed_types
            .contains(&TypeRef::Named("Array".into()))
    );
    assert!(
        !free_fn
            .constructed_types
            .contains(&TypeRef::Named("Order".into()))
    );
}

#[test]
fn lexical_shadowing_handles_bindings_cases_catch_captures_and_positive_controls() {
    let src = r#"
struct Account {
    var customer: Customer?
    var total: Int
    var bonus: Int
    var error: Error?

    func validate(_ n: Int) -> Bool { n > 0 }

    func shadowBasics(items: [Account]) -> String {
        let total = total
        for bonus in items { _ = bonus }
        _ = items.map { total in total + 1 }
        _ = items.map { [total = self.total, weak bonus] x in total + bonus + x }
        _ = items.map(\.total)
        _ = items.map(\.customer.name)
        _ = \Account.bonus
        if let customer = customer { return customer.name }
        if let customer { return customer.name }
        guard let customer = customer else { return String(describing: customer) }
        return "\(total)-\(customer.name)"
    }

    func shadowCasesAndCatch(state: State, opt: Int?, validate: (Int) -> Bool) -> Int {
        var acc = 0
        if case let .loaded(total) = state { acc += total }
        if case .pair(let total, let bonus) = state { acc += total + bonus }
        if case let total? = opt { acc += total }
        guard case let .loaded(bonus) = state else { return 0 }
        acc += bonus
        switch state {
        case let .loaded(total): acc += total
        case let (total, bonus): acc += total + bonus
        case .pair(let total, var bonus): acc += total + bonus
        default: break
        }
        while let total = nextInt() { acc += total }
        do { try work() } catch MyError.failed(let total) { acc += total } catch { self.error = error }
        func helper(total: Int) -> Int { return total + 1 }
        if validate(acc) { acc = helper(total: acc) }
        return acc
    }

    func positiveUnboundControls(state: State) -> Int {
        var acc = 0
        switch state {
        case .loaded(total): acc += 1
        default: break
        }
        do { try work() } catch is MyError { _ = error }
        return acc
    }
}
"#;
    let (_, ast) = extract_swift("Sources/Shop/Account.swift", src);

    let basics = find_fn(&ast, "shadowBasics");
    assert_eq!(count_access(basics, "self", "total", None), 2);
    assert_eq!(count_access(basics, "self", "bonus", None), 1);
    assert_eq!(count_access(basics, "self", "customer", None), 4);
    assert_eq!(count_access(basics, "customer", "name", None), 3);

    let advanced = find_fn(&ast, "shadowCasesAndCatch");
    assert_eq!(count_access(advanced, "self", "total", None), 0);
    assert_eq!(count_access(advanced, "self", "bonus", None), 0);
    assert_eq!(count_access(advanced, "self", "error", Some(false)), 0);
    assert_eq!(count_access(advanced, "self", "error", Some(true)), 1);
    assert!(
        advanced
            .calls
            .iter()
            .any(|c| c.callee == "validate" && c.kind == CallKind::Free && c.receiver.is_none())
    );
    assert!(
        !advanced
            .calls
            .iter()
            .any(|c| c.callee == "validate" && c.receiver == Some(CallReceiver::SelfInstance))
    );

    let controls = find_fn(&ast, "positiveUnboundControls");
    assert_eq!(count_access(controls, "self", "total", Some(false)), 1);
    assert_eq!(count_access(controls, "self", "error", Some(false)), 1);
}

#[test]
fn extracts_xctest_void_rule_suite_classes_cross_file_ext_fixtures_and_yaml() {
    let test_src = r#"
import XCTest
@testable import Shop

@Suite
final class SuiteOrderTests {
    var sut = /* default */ Order(total: 10)
    init() { _ = Order(total: 1) }
    deinit { cleanup() }

    func testHelperNotAnnotated() {}

    @Test("applies discount")
    func appliesDiscount() {
        _ = sut.total
    }

    struct Fixture {
        var total: Int = 0
        func readTotal() -> Int { total }
    }
}

final class LegacyOrderTests: XCTestCase {
    func testLegacyCase() { let x = 1; _ = x }
    func testExplicitVoid() -> Void { let x = 1; _ = x }
    func testTupleVoid() -> () { let x = 1; _ = x }
    func testBuilderHelper() -> Order { Order(total: 5) }
}

extension LegacyOrderTests {
    func testSameFileExtendedCase() { let y = 2; _ = y }
}
"#;
    let (test_tree, test_ast) = extract_swift("Tests/ShopTests/OrderTests.swift", test_src);
    let role_of = |name: &str| find_fn(&test_ast, name).role;
    assert_eq!(role_of("testHelperNotAnnotated"), FunctionRole::TestHelper);
    assert_eq!(role_of("appliesDiscount"), FunctionRole::Test);
    assert_eq!(role_of("testLegacyCase"), FunctionRole::Test);
    assert_eq!(role_of("testExplicitVoid"), FunctionRole::Test);
    assert_eq!(role_of("testTupleVoid"), FunctionRole::Test);
    assert_eq!(role_of("testBuilderHelper"), FunctionRole::TestHelper);
    assert_eq!(role_of("testSameFileExtendedCase"), FunctionRole::Test);

    let sut_init = find_fn(&test_ast, "sut");
    assert_eq!(sut_init.role, FunctionRole::TestHelper);
    assert!(
        sut_init
            .constructed_types
            .contains(&TypeRef::Named("Order".into()))
    );
    let suite_init = find_fn(&test_ast, "init");
    assert_eq!(suite_init.role, FunctionRole::TestHelper);
    assert!(
        suite_init
            .constructed_types
            .contains(&TypeRef::Named("Order".into()))
    );
    let suite_deinit = find_fn(&test_ast, "deinit");
    assert_eq!(suite_deinit.role, FunctionRole::TestHelper);
    assert!(suite_deinit.calls.iter().any(|c| c.callee == "cleanup"));

    assert!(
        test_ast
            .types
            .iter()
            .any(|t| t.name == "SuiteOrderTests::Fixture")
    );
    let read_total = find_fn(&test_ast, "readTotal");
    assert_eq!(
        read_total.owner_type.as_deref(),
        Some("SuiteOrderTests::Fixture")
    );
    assert_eq!(count_access(read_total, "self", "total", Some(false)), 1);

    let (_, ext_ast) = extract_swift(
        "Tests/ShopTests/OrderRefundTests.swift",
        "extension LegacyOrderTests { func testCrossFileRefund() { let z = 3; _ = z } }",
    );
    assert_eq!(
        find_fn(&ext_ast, "testCrossFileRefund").role,
        FunctionRole::Test
    );

    let (prod_tree, prod_ast) =
        extract_swift("Sources/Shop/Loader.swift", "func f() { _ = try! g() }");
    let codebase = Codebase::build(vec![prod_tree, test_tree], vec![prod_ast, test_ast]);

    let mut registry = RuleRegistry::empty();
    let yaml = r"
id: no-swift-force-try
language: swift
severity: error
message: Avoid force-try in Swift
rule:
  pattern: try! $EXPR
";
    let loaded = registry
        .load_ast_grep_yaml(yaml, &LanguageRegistry::with_defaults())
        .unwrap_or_else(|err| panic!("swift YAML rule failed to compile: {err}"));
    assert_eq!(loaded, 1);

    let diags = registry.run(&codebase, &Config::default());
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].rule_id(), "no-swift-force-try");
}
