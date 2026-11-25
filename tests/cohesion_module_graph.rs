mod support;

use rqcode::ast::codebase::Codebase;
use rqcode::ast::item::{ParamAccess, Visibility};
use rqcode::ast::{ModulePath, SyntaxTree};
use rqcode::lang::rust::{RustFrontend, rust_language_spec};
use rqcode::lang::swift::{SwiftFrontend, swift_language_spec};
use rqcode::lang::typescript::{TypeScriptFrontend, typescript_language_spec};
use rqcode::lang::{LanguageFrontend, LanguageRegistry};
use rqcode::rules::RuleRegistry;
use std::error::Error;
use std::path::Path;
use support::{analyze_project, assert_no_rule, cohesion_only_config, diags_for};

// ---------------------------------------------------------------------------
// Group D: ModuleGraph (§4.3)
// ---------------------------------------------------------------------------

#[test]
fn module_graph_detects_named_and_wildcard_reexport_cycles_with_export_span()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    // 1. Named TS re-export cycle — verify span points to line 2 (`export { ... } from`)
    let ts_named = analyze_project(
        &[
            ("src/a.ts", "export class A {}\nexport { B } from './b';\n"),
            ("src/b.ts", "export class B {}\nexport { A } from './a';\n"),
        ],
        &cfg,
    )?;
    let named_cycles = diags_for(&ts_named, "cyclic-module-dependency");
    assert_eq!(named_cycles.len(), 1);
    assert_eq!(
        named_cycles[0].span().start_line(),
        2,
        "re-export cycle span must point to ExportDecl line 2, got: {:?}",
        named_cycles[0].span()
    );

    // 2. Wildcard + namespace re-export cycle (`export * from` / `export * as ns from`)
    let ts_wild = analyze_project(
        &[
            ("src/a.ts", "export class A {}\nexport * from './b';\n"),
            (
                "src/b.ts",
                "export class B {}\nexport * as nsA from './a';\n",
            ),
        ],
        &cfg,
    )?;
    assert_eq!(diags_for(&ts_wild, "cyclic-module-dependency").len(), 1);

    // 3. Rust submodule-first resolution on `pub use crate::a::sub;` (NO spurious cycle!)
    let rust_no_cycle = analyze_project(
        &[
            ("src/lib.rs", "pub mod a;\npub mod b;\n"),
            (
                "src/a/mod.rs",
                "pub mod sub;\nuse crate::b::B;\npub fn make() -> B { B }\n",
            ),
            ("src/a/sub.rs", "pub struct Leaf;\n"),
            ("src/b.rs", "pub struct B;\npub use crate::a::sub;\n"),
        ],
        &cfg,
    )?;
    assert_no_rule(&rust_no_cycle, "cyclic-module-dependency");
    Ok(())
}

#[test]
fn module_graph_merges_multi_file_paths_and_excludes_test_files_and_reexport_ancestors()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    // 1. Multi-file merge: `src/alpha.ts` and `src/alpha/index.ts` share ModulePath(["src", "alpha"]).
    //    alpha.ts -> beta -> alpha AND alpha/index.ts -> gamma -> alpha form one 3-module SCC.
    let merged = analyze_project(
        &[
            (
                "src/alpha.ts",
                "import { Beta } from './beta';\nexport function fromAlpha(): Beta { return new Beta(); }\n",
            ),
            (
                "src/alpha/index.ts",
                "import { Gamma } from '../gamma';\nexport function alphaEntry(): Gamma { return new Gamma(); }\n",
            ),
            (
                "src/beta.ts",
                "import { fromAlpha } from './alpha';\nexport class Beta { v(): unknown { return fromAlpha(); } }\n",
            ),
            (
                "src/gamma.ts",
                "import { alphaEntry } from './alpha/index';\nexport class Gamma { v(): unknown { return alphaEntry(); } }\n",
            ),
        ],
        &cfg,
    )?;
    let cycles = diags_for(&merged, "cyclic-module-dependency");
    assert_eq!(
        cycles.len(),
        1,
        "expected 1 merged 3-module SCC, got: {merged:?}"
    );
    let msg = cycles[0].message();
    assert!(
        msg.contains("src::alpha") && msg.contains("src::beta") && msg.contains("src::gamma"),
        "merged SCC must include src::alpha, src::beta, and src::gamma, got: {msg}"
    );

    // 2. Test file sharing ModulePath with production file must NOT create a cycle
    let test_file_no_cycle = analyze_project(
        &[
            ("src/order.ts", "export class Order {}\n"),
            (
                "src/order.test.ts",
                "import { Billing } from './billing';\ntest('t', () => { new Billing(); });\n",
            ),
            (
                "src/billing.ts",
                "import { Order } from './order';\nexport class Billing { make(): Order { return new Order(); } }\n",
            ),
        ],
        &cfg,
    )?;
    assert_no_rule(&test_file_no_cycle, "cyclic-module-dependency");

    // 3. Ancestor/child re-exports (`models/index.ts` <-> `models/order.ts`) must NOT create a cycle
    let ancestor_reexport = analyze_project(
        &[
            (
                "src/models/index.ts",
                "export class Registry {}\nexport { Order } from './order';\n",
            ),
            (
                "src/models/order.ts",
                "export class Order {}\nexport { Registry } from './index';\n",
            ),
        ],
        &cfg,
    )?;
    assert_no_rule(&ancestor_reexport, "cyclic-module-dependency");
    Ok(())
}

fn parse_rust_mod(
    rel: &str,
    segs: &[&str],
    import_target: Option<&[&str]>,
    src: &str,
) -> (SyntaxTree, rqcode::ast::FileAst) {
    let tree = SyntaxTree::parse(rel, rel, rust_language_spec(), src);
    let mut ast = RustFrontend.extract(&tree, Path::new("."));
    ast.module_path = ModulePath::new(segs.iter().map(|s| (*s).to_owned()).collect());
    if let Some(target) = import_target {
        ast.imports[0].resolved_module =
            Some(ModulePath::new(target.iter().map(|s| (*s).to_owned()).collect()));
    }
    (tree, ast)
}

fn parse_ts_mod(
    rel: &str,
    segs: &[&str],
    import_target: Option<&[&str]>,
    src: &str,
) -> (SyntaxTree, rqcode::ast::FileAst) {
    let tree = SyntaxTree::parse(rel, rel, typescript_language_spec(), src);
    let mut ast = TypeScriptFrontend.extract(&tree, Path::new("."));
    ast.module_path = ModulePath::new(segs.iter().map(|s| (*s).to_owned()).collect());
    if let Some(target) = import_target {
        ast.imports[0].resolved_module =
            Some(ModulePath::new(target.iter().map(|s| (*s).to_owned()).collect()));
    }
    (tree, ast)
}

#[test]
fn module_graph_partitions_by_language_id_even_with_colliding_paths() -> Result<(), Box<dyn Error>>
{
    let cfg = cohesion_only_config();
    let registry =
        RuleRegistry::from_config(&cfg, Path::new("."), &LanguageRegistry::with_defaults())?;

    // Case 1: colliding ModulePaths across Rust & TS (Rust a->b, TS b->a) -> 0 cycles
    let items_acyclic = [
        parse_rust_mod("shared/a.rs", &["shared", "a"], Some(&["shared", "b"]), "use crate::b::B;
pub struct A;
"),
        parse_rust_mod("shared/b.rs", &["shared", "b"], None, "pub struct B;
"),
        parse_ts_mod("shared/a.ts", &["shared", "a"], None, "export class A {}
"),
        parse_ts_mod("shared/b.ts", &["shared", "b"], Some(&["shared", "a"]), "import { A } from './a';
export class B {}
"),
    ];
    let (trees, asts): (Vec<_>, Vec<_>) = items_acyclic.into_iter().unzip();
    let acyclic_diags = registry.run(&Codebase::build(trees, asts), &cfg);
    assert_no_rule(&acyclic_diags, "cyclic-module-dependency");

    // Case 2: real Rust cycle (a<->b) alongside colliding acyclic TS files -> 1 cycle
    let items_cyclic = [
        parse_rust_mod("shared/a.rs", &["shared", "a"], Some(&["shared", "b"]), "use crate::b::B;
pub struct A;
"),
        parse_rust_mod("shared/b.rs", &["shared", "b"], Some(&["shared", "a"]), "use crate::a::A;
pub struct B;
"),
        parse_ts_mod("shared/a.ts", &["shared", "a"], None, "export class A {}
"),
        parse_ts_mod("shared/b.ts", &["shared", "b"], None, "export class B {}
"),
    ];
    let (trees, asts): (Vec<_>, Vec<_>) = items_cyclic.into_iter().unzip();
    let cyclic_diags = registry.run(&Codebase::build(trees, asts), &cfg);
    assert_eq!(diags_for(&cyclic_diags, "cyclic-module-dependency").len(), 1);
    Ok(())
}

// ---------------------------------------------------------------------------
// Group E: Frontend Extraction Invariants (explicit_visibility & Swift class receivers)
// ---------------------------------------------------------------------------

#[test]
fn frontends_set_explicit_visibility_and_swift_class_receivers_accurately() {
    let rust_tree = SyntaxTree::parse(
        "src/m.rs",
        "src/m.rs",
        rust_language_spec(),
        "pub struct S { priv_field: i32, pub pub_field: i32, pub(crate) crate_field: i32 }\n",
    );
    let rust_ast = RustFrontend.extract(&rust_tree, Path::new("."));
    let s = &rust_ast.types[0];
    assert_eq!(s.fields[0].visibility, Visibility::Private);
    assert!(!s.fields[0].explicit_visibility);
    assert_eq!(s.fields[1].visibility, Visibility::Public);
    assert!(s.fields[1].explicit_visibility);
    assert_eq!(s.fields[2].visibility, Visibility::Internal);
    assert!(s.fields[2].explicit_visibility);

    let ts_tree = SyntaxTree::parse(
        "src/m.ts",
        "src/m.ts",
        typescript_language_spec(),
        r"
export class C {
    defaultPub: number = 0;
    public explicitPub: number = 0;
    protected explicitProt: number = 0;
    private explicitPriv: number = 0;
}
",
    );
    let ts_ast = TypeScriptFrontend.extract(&ts_tree, Path::new("."));
    let c = &ts_ast.types[0];
    assert_eq!(c.fields[0].visibility, Visibility::Public);
    assert!(!c.fields[0].explicit_visibility);
    assert_eq!(c.fields[1].visibility, Visibility::Public);
    assert!(c.fields[1].explicit_visibility);
    assert_eq!(c.fields[2].visibility, Visibility::Internal);
    assert!(c.fields[2].explicit_visibility);
    assert_eq!(c.fields[3].visibility, Visibility::Private);
    assert!(c.fields[3].explicit_visibility);

    let swift_tree = SyntaxTree::parse(
        "Sources/App/Model.swift",
        "Sources/App/Model.swift",
        swift_language_spec(),
        r"
public class Model {
    var defaultInternal: Int = 0
    internal var explicitInternal: Int = 0
    public var explicitPublic: Int = 0
    private var explicitPrivate: Int = 0

    public func bodyMethod() -> Int { return defaultInternal }
}

extension Model {
    public func extMethod() -> Int { return explicitPublic }
}
",
    );
    let swift_ast = SwiftFrontend::new().extract(&swift_tree, Path::new("."));
    let m = &swift_ast.types[0];
    assert_eq!(m.fields[0].visibility, Visibility::Internal);
    assert!(!m.fields[0].explicit_visibility);
    assert_eq!(m.fields[1].visibility, Visibility::Internal);
    assert!(m.fields[1].explicit_visibility);
    assert_eq!(m.fields[2].visibility, Visibility::Public);
    assert!(m.fields[2].explicit_visibility);
    assert_eq!(m.fields[3].visibility, Visibility::Private);
    assert!(m.fields[3].explicit_visibility);

    let body_fn = swift_ast
        .functions
        .iter()
        .find(|f| f.name == "bodyMethod")
        .unwrap_or_else(|| panic!("missing bodyMethod"));
    assert_eq!(body_fn.self_receiver, Some(ParamAccess::MutableRef));

    let ext_fn = swift_ast
        .functions
        .iter()
        .find(|f| f.name == "extMethod")
        .unwrap_or_else(|| panic!("missing extMethod"));
    assert!(ext_fn.has_self_receiver());
}
