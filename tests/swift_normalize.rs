use rqcode::ast::item::FunctionDecl;
use rqcode::ast::similarity::{
    AstKind, EligibilityConfig, NameSpace, NodeLabel, NormalizedTree, SimilarityCorpus,
};
use rqcode::ast::{FileAst, SyntaxTree};
use rqcode::lang::LanguageFrontend;
use rqcode::lang::swift::{SwiftFrontend, swift_language_spec};
use std::path::Path;

fn extract_swift(rel_path: &str, src: &str) -> FileAst {
    let tree = SyntaxTree::parse(rel_path, rel_path, swift_language_spec(), src);
    SwiftFrontend::new().extract(&tree, Path::new("."))
}

fn find_fn<'a>(ast: &'a FileAst, name: &str) -> &'a FunctionDecl {
    ast.functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("missing function `{name}`"))
}

fn has_symbol(tree: &NormalizedTree, space: NameSpace, expected: &str) -> bool {
    tree.nodes().iter().any(|n| {
        matches!(
            n.label(),
            NodeLabel::Symbol { space: s, name } if *s == space && name == expected
        )
    })
}

fn has_ast_kind(tree: &NormalizedTree, kind: AstKind) -> bool {
    tree.nodes()
        .iter()
        .any(|n| n.label() == &NodeLabel::Struct(kind))
}

fn has_exact(tree: &NormalizedTree, token: &str) -> bool {
    tree.nodes()
        .iter()
        .any(|n| matches!(n.label(), NodeLabel::Exact(t) if t == token))
}

#[test]
fn normalizes_swift_clones_implicit_self_shadowing_and_switch_cases() {
    let src = r##"
struct Calculator {
    var factor: Double
    var onTap: () -> Void

    func helper(_ value: Double) -> Double { value * 2.0 }

    func explicitSelf(prices: [Double], threshold: Double) -> Double {
        var acc = 0.0
        for price in prices {
            if price > threshold { acc += self.helper(price) * self.factor }
        }
        self.onTap()
        return acc
    }

    func implicitSelf(amounts: [Double], cutoff: Double) -> Double {
        var sum = 0.0
        for amount in amounts {
            if amount > cutoff { sum += helper(amount) * factor }
        }
        onTap()
        return sum
    }

    func guardedShadowA(state: State) -> Double {
        guard case let .loaded(factor) = state else { return 0.0 }
        if factor > 0.0 { return factor * 2.0 + 1.0 }
        return 0.0
    }

    func guardedShadowB(input: State) -> Double {
        guard case let .loaded(value) = input else { return 0.0 }
        if value > 0.0 { return value * 2.0 + 1.0 }
        return 0.0
    }
}

enum Status { case active; case inactive }

func emitSuccess(v: Int, flag: Bool, s: Status) {
    if flag && s == Status.active { complete(.success(v)); logResult(true, #"ok"#) } else { complete(.success(0)); logResult(false, #"no"#) }
}
func emitFailure(v: Int, flag: Bool, s: Status) {
    if flag && s == Status.inactive { complete(.failure(v)); logResult(true, #"ok"#) } else { complete(.failure(0)); logResult(false, #"no"#) }
}

struct BigRecord {
    let a: Int; let b: Int; let c: Int; let d: Int; let e: Int; let f: Int
    init(a: Int, b: Int, c: Int, d: Int, e: Int, f: Int) {
        self.a = a; self.b = b; self.c = c; self.d = d; self.e = e; self.f = f
    }
}
"##;
    let ast = extract_swift("Sources/Shop/Billing.swift", src);
    let mut corpus = SimilarityCorpus::new(EligibilityConfig {
        min_nodes: 15,
        min_distinct_kinds: 4,
    });
    assert!(corpus.add_unit(0, &find_fn(&ast, "explicitSelf").body_tree));
    assert!(corpus.add_unit(1, &find_fn(&ast, "implicitSelf").body_tree));
    assert!(!corpus.add_unit(2, &find_fn(&ast, "init").body_tree));
    assert!(corpus.add_unit(3, &find_fn(&ast, "emitSuccess").body_tree));
    assert!(corpus.add_unit(4, &find_fn(&ast, "emitFailure").body_tree));
    assert!(corpus.add_unit(5, &find_fn(&ast, "guardedShadowA").body_tree));
    assert!(corpus.add_unit(6, &find_fn(&ast, "guardedShadowB").body_tree));

    let matches = corpus.similar_pairs(0.85);
    assert_eq!(
        matches.len(),
        2,
        "expected (0, 1) implicit-self clone and (5, 6) shadowed guard-case clone: {matches:?}"
    );
    let pairs: Vec<(usize, usize)> = matches
        .iter()
        .map(|m| (m.left_id(), m.right_id()))
        .collect();
    assert!(pairs.contains(&(0, 1)));
    assert!(pairs.contains(&(5, 6)));
    for m in &matches {
        assert!((m.score() - 1.0).abs() < 1e-12);
    }
}

#[test]
fn swift_switch_patterns_and_nested_type_paths_preserve_case_names() {
    let src = r"
func routeA(s: Status, p: Payment.Status, v: Int) -> Int {
    if p == Payment.Status.paid { return compute(v + 1) }
    switch s {
    case Status.active: return compute(v + 1)
    case .inactive: return fallback(v - 1)
    }
}

func routeARenamed(x: Status, q: Payment.Status, n: Int) -> Int {
    if q == Payment.Status.paid { return compute(n + 1) }
    switch x {
    case Status.active: return compute(n + 1)
    case .inactive: return fallback(n - 1)
    }
}

func routeSwapped(s: Status, p: Payment.Status, v: Int) -> Int {
    if p == Payment.Status.refunded { return compute(v + 1) }
    switch s {
    case Status.inactive: return compute(v + 1)
    case .active: return fallback(v - 1)
    }
}

func nestedPaid(p: Payment.Status) -> Bool { p == Payment.Status.paid }
func nestedRefunded(p: Payment.Status) -> Bool { p == Payment.Status.refunded }
";
    let ast = extract_swift("Sources/Shop/Router.swift", src);
    let paid_tree = &find_fn(&ast, "nestedPaid").body_tree;
    let refunded_tree = &find_fn(&ast, "nestedRefunded").body_tree;
    assert!(has_exact(paid_tree, "Payment"));
    assert!(has_exact(paid_tree, "Status"));
    assert!(has_exact(paid_tree, "paid"));
    assert!(has_exact(refunded_tree, "refunded"));
    assert_ne!(paid_tree.nodes(), refunded_tree.nodes());

    let mut corpus = SimilarityCorpus::new(EligibilityConfig {
        min_nodes: 10,
        min_distinct_kinds: 4,
    });
    assert!(corpus.add_unit(0, &find_fn(&ast, "routeA").body_tree));
    assert!(corpus.add_unit(1, &find_fn(&ast, "routeARenamed").body_tree));
    assert!(corpus.add_unit(2, &find_fn(&ast, "routeSwapped").body_tree));

    let matches = corpus.similar_pairs(0.85);
    assert_eq!(matches.len(), 1, "unexpected matches: {matches:?}");
    assert_eq!((matches[0].left_id(), matches[0].right_id()), (0, 1));
    assert!((matches[0].score() - 1.0).abs() < 1e-12);
}

#[test]
fn normalizer_handles_switch_catch_nested_func_shorthand_keypaths_subscripts_and_captures() {
    let src = r#"
struct Account {
    var total: Int?
    var bonus: Int
    var error: Error?
    var items: [Int]
    var label: String

    func switchShadowed(s: State) -> Int {
        switch s {
        case let .loaded(total): return total + 1
        default: return 0
        }
    }
    func switchUnbound(s: State) -> Int {
        switch s {
        case .loaded(bonus): return bonus + 1
        default: return 0
        }
    }

    func catchImplicitError() {
        do { try work() } catch { log(error) }
    }
    func catchExplicitError() {
        do { try work() } catch let error as MyError { log(error) }
    }
    func catchExplicitOther() {
        do { try work() } catch let failure { _ = failure; log(error) }
    }
    func catchSelfError() {
        do { try work() } catch is MyError { log(error) }
    }

    func nestedFuncShadow(x: Int) -> Int {
        func inner(bonus: Int) -> Int { return bonus + 1 }
        return inner(bonus: x)
    }

    func ifLetShorthand() -> Int {
        if let total { return total + 1 }
        return 0
    }
    func ifLetExplicitSelf() -> Int {
        if let total = self.total { return total + 1 }
        return 0
    }

    func keyPathImplicit(xs: [Account]) -> [Int?] { xs.map(\.total) }
    func keyPathExplicit(xs: [Account]) -> [Int] { xs.map(\Account.bonus) }
    func subscriptSelf(i: Int) -> Int { self.items[i] }
    func subscriptImplicit(i: Int) -> Int { items[i] }
    func trailingLabelA() { Panel { save() } label: { Caption("Hi") } }
    func captureShadow(xs: [Int]) -> [Int] { xs.map { [total = self.total] x in x + (total ?? 0) } }
}
"#;
    let ast = extract_swift("Sources/Shop/NormCases.swift", src);
    let tree_of = |name: &str| &find_fn(&ast, name).body_tree;
    let has_self = |tree: &NormalizedTree| has_symbol(tree, NameSpace::Plain, "self");

    assert!(!has_self(tree_of("switchShadowed")));
    assert!(has_self(tree_of("switchUnbound")));

    assert!(!has_self(tree_of("catchImplicitError")));
    assert!(!has_self(tree_of("catchExplicitError")));
    assert!(has_self(tree_of("catchExplicitOther")));
    assert!(has_self(tree_of("catchSelfError")));

    let nested = tree_of("nestedFuncShadow");
    assert!(has_ast_kind(nested, AstKind::Closure));
    assert!(!has_self(nested));

    assert_eq!(
        tree_of("ifLetShorthand").nodes(),
        tree_of("ifLetExplicitSelf").nodes()
    );

    let kp_implicit = tree_of("keyPathImplicit");
    let kp_explicit = tree_of("keyPathExplicit");
    assert!(has_exact(kp_implicit, "\\"));
    assert!(has_symbol(kp_implicit, NameSpace::Member, "total"));
    assert!(!has_self(kp_implicit));
    assert!(has_exact(kp_explicit, "Account"));
    assert!(has_symbol(kp_explicit, NameSpace::Member, "bonus"));
    assert!(!has_self(kp_explicit));

    let sub_self = tree_of("subscriptSelf");
    assert!(has_ast_kind(sub_self, AstKind::Index));
    assert_eq!(sub_self.nodes(), tree_of("subscriptImplicit").nodes());

    let trailing = tree_of("trailingLabelA");
    assert!(!has_self(trailing));
    assert!(has_symbol(trailing, NameSpace::Label, "label"));

    let capture = tree_of("captureShadow");
    assert!(has_self(capture));
    assert!(has_symbol(capture, NameSpace::Plain, "total"));
    assert_eq!(
        capture
            .nodes()
            .iter()
            .filter(|n| matches!(
                n.label(),
                NodeLabel::Symbol {
                    space: NameSpace::Plain,
                    name
                } if name == "self"
            ))
            .count(),
        1,
        "capture list must read `self.total` once and shadow `total` inside the closure body"
    );
}
