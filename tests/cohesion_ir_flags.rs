use rqcode::ast::SyntaxTree;
use rqcode::ast::item::ExportOrigin;
use rqcode::lang::LanguageFrontend;
use rqcode::lang::rust::{RustFrontend, rust_language_spec};
use rqcode::lang::swift::{SwiftFrontend, swift_language_spec};
use rqcode::lang::typescript::{TypeScriptFrontend, typescript_language_spec};
use std::path::Path;

#[test]
fn frontends_set_function_is_lifecycle_accurately() {
    let ts_tree = SyntaxTree::parse(
        "src/order.ts",
        "src/order.ts",
        typescript_language_spec(),
        r"
export class Order {
    cents: number;
    constructor(cents: number) { this.cents = cents; }
    total(): number { return this.cents; }
}
",
    );
    let ts_ast = TypeScriptFrontend::new().extract(&ts_tree, Path::new("."));
    let ctor = ts_ast
        .functions
        .iter()
        .find(|f| f.name == "constructor")
        .unwrap_or_else(|| panic!("missing TS constructor"));
    let total = ts_ast
        .functions
        .iter()
        .find(|f| f.name == "total")
        .unwrap_or_else(|| panic!("missing TS total"));
    assert!(ctor.is_lifecycle);
    assert!(!total.is_lifecycle);

    let swift_tree = SyntaxTree::parse(
        "Sources/Shop/Order.swift",
        "Sources/Shop/Order.swift",
        swift_language_spec(),
        r"
public class Order {
    public var cents: Int
    public init(cents: Int) { self.cents = cents }
    deinit {}
    public func total() -> Int { return cents }
}
",
    );
    let swift_ast = SwiftFrontend::new().extract(&swift_tree, Path::new("."));
    let init_fn = swift_ast
        .functions
        .iter()
        .find(|f| f.name == "init")
        .unwrap_or_else(|| panic!("missing Swift init"));
    let deinit_fn = swift_ast
        .functions
        .iter()
        .find(|f| f.name == "deinit")
        .unwrap_or_else(|| panic!("missing Swift deinit"));
    let swift_total = swift_ast
        .functions
        .iter()
        .find(|f| f.name == "total")
        .unwrap_or_else(|| panic!("missing Swift total"));
    assert!(init_fn.is_lifecycle);
    assert!(deinit_fn.is_lifecycle);
    assert!(!swift_total.is_lifecycle);

    let rust_tree = SyntaxTree::parse(
        "src/order.rs",
        "src/order.rs",
        rust_language_spec(),
        "pub struct Order { pub cents: u64 }\nimpl Order { pub fn new(cents: u64) -> Self { Self { cents } } pub fn total(&self) -> u64 { self.cents } }\n",
    );
    let rust_ast = RustFrontend.extract(&rust_tree, Path::new("."));
    assert!(rust_ast.functions.iter().all(|f| !f.is_lifecycle));
}

#[test]
fn frontends_set_type_is_immutable_record_accurately() {
    let ts_tree = SyntaxTree::parse(
        "src/records.ts",
        "src/records.ts",
        typescript_language_spec(),
        r"
export class ReadonlyBody {
    private static version: number = 1;
    readonly id: number = 0;
    readonly cents: number = 0;
}

export class BareReadonlyParam {
    constructor(readonly id: number) {}
}

export class StaticOnly {
    static readonly VERSION: number = 1;
}

export class MixedFields {
    readonly id: number = 0;
    cents: number = 0;
}

export class MutableParam {
    constructor(public readonly id: number, public cents: number) {}
}
",
    );
    let ts_ast = TypeScriptFrontend::new().extract(&ts_tree, Path::new("."));
    let by_name = |name: &str| {
        ts_ast
            .types
            .iter()
            .find(|t| t.name == name)
            .unwrap_or_else(|| panic!("missing TS class `{name}`"))
    };
    assert!(by_name("ReadonlyBody").is_immutable_record);
    assert!(by_name("BareReadonlyParam").is_immutable_record);
    assert!(!by_name("StaticOnly").is_immutable_record);
    assert!(!by_name("MixedFields").is_immutable_record);
    assert!(!by_name("MutableParam").is_immutable_record);
    assert!(!by_name("ReadonlyBody").has_implicit_field_readers);

    let rust_tree = SyntaxTree::parse(
        "src/dto.rs",
        "src/dto.rs",
        rust_language_spec(),
        "#[derive(Serialize)]\npub struct RustDto { pub id: u64 }\n",
    );
    let rust_ast = RustFrontend.extract(&rust_tree, Path::new("."));
    assert!(!rust_ast.types[0].is_immutable_record);
    assert!(rust_ast.types[0].has_implicit_field_readers);

    let swift_tree = SyntaxTree::parse(
        "Sources/Shop/Dto.swift",
        "Sources/Shop/Dto.swift",
        swift_language_spec(),
        "public struct SwiftDto: Codable { public let id: Int }\n",
    );
    let swift_ast = SwiftFrontend::new().extract(&swift_tree, Path::new("."));
    assert!(!swift_ast.types[0].is_immutable_record);
    assert!(swift_ast.types[0].has_implicit_field_readers);
}

#[test]
fn frontends_set_export_origin_and_span_for_direct_internal_reexports() {
    let ts_tree = SyntaxTree::parse(
        "src/a.ts",
        "src/a.ts",
        typescript_language_spec(),
        "export class A {}\nexport { B } from './b';\n",
    );
    let ts_ast = TypeScriptFrontend::new().extract(&ts_tree, Path::new("."));
    let reexport = ts_ast
        .exports
        .iter()
        .find(|e| e.origin == ExportOrigin::DirectInternal)
        .unwrap_or_else(|| panic!("missing TS DirectInternal export: {:?}", ts_ast.exports));
    assert_eq!(reexport.span.start_line(), 2);

    let rust_tree = SyntaxTree::parse(
        "src/a.rs",
        "src/a.rs",
        rust_language_spec(),
        "pub struct A;\npub use crate::b::B;\n",
    );
    let rust_ast = RustFrontend.extract(&rust_tree, Path::new("."));
    let rust_reexport = rust_ast
        .exports
        .iter()
        .find(|e| e.origin == ExportOrigin::DirectInternal)
        .unwrap_or_else(|| panic!("missing Rust DirectInternal export: {:?}", rust_ast.exports));
    assert_eq!(rust_reexport.span.start_line(), 2);
}
