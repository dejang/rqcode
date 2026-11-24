use rqcode::Analyzer;
use rqcode::ast::SyntaxTree;
use rqcode::ast::item::CallKind;
use rqcode::ast::similarity::NodeLabel;
use rqcode::config::Config;
use rqcode::diagnostic::Diagnostic;
use rqcode::lang::LanguageFrontend;
use rqcode::lang::rust::{RustFrontend, rust_language_spec};
use rqcode::report::Report;
use rqcode::source::{SourceFile, SourceTree};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

fn polyglot_cases() -> [(&'static str, &'static str, &'static str, &'static str); 3] {
    [
        (
            "src/billing.rs",
            include_str!("fixtures/similarity/billing.rs"),
            "compute_invoice_total",
            "compute_cart_subtotal",
        ),
        (
            "src/billing.ts",
            include_str!("fixtures/similarity/billing.ts"),
            "scoreCart",
            "scoreOrder",
        ),
        (
            "Sources/Ledger.swift",
            include_str!("fixtures/similarity/Ledger.swift"),
            "totalPositive",
            "sumValid",
        ),
    ]
}

fn similarity_only_config() -> Config {
    let mut config = Config::default();
    config.cohesion.enabled = false;
    config.test_coupling.enabled = false;
    config.ast_grep.enabled = false;
    config
}

fn check_single_file(path: &str, src: &str, config: &Config) -> Report {
    let analyzer = Analyzer::default();
    let Some(frontend) = analyzer.languages().frontend_for_path(Path::new(path)) else {
        panic!("missing frontend for {path}");
    };
    let sources = SourceTree::from_files(
        ".",
        vec![SourceFile::new(
            PathBuf::from(path),
            PathBuf::from(path),
            frontend.grammar(),
            src,
        )],
    );
    analyzer.check_sources(&sources, config)
}

#[test]
fn polyglot_detects_duplicate_functions_and_overlapping_tests_at_default_thresholds() {
    let config = similarity_only_config();
    let mut cases: Vec<(&str, &str, &str, &str, &str)> = polyglot_cases()
        .into_iter()
        .map(|(path, src, left, right)| ("duplicate-function", path, src, left, right))
        .collect();
    cases.extend([
        (
            "overlapping-tests",
            "tests/ledger_tests.rs",
            include_str!("fixtures/similarity/ledger_tests.rs"),
            "test_invoice_total_with_cap",
            "test_cart_subtotal_with_ceiling",
        ),
        (
            "overlapping-tests",
            "src/ledger.test.ts",
            include_str!("fixtures/similarity/ledger.test.ts"),
            "invoice total with cap",
            "cart subtotal with ceiling",
        ),
        (
            "overlapping-tests",
            "Tests/ShopTests/LedgerTests.swift",
            include_str!("fixtures/similarity/LedgerTests.swift"),
            "testInvoiceTotalWithCap",
            "testCartSubtotalWithCeiling",
        ),
    ]);

    let mut matched = Vec::new();
    for (rule_id, path, src, expected_left, expected_right) in cases {
        let report = check_single_file(path, src, &config);
        matched.clear();
        for diag in report.diagnostics() {
            if diag.rule_id() == rule_id {
                matched.push(diag.message().to_string());
            }
        }
        assert_eq!(
            matched.len(),
            1,
            "{path}: expected 1 `{rule_id}` ({expected_left}, {expected_right}), got {matched:?}"
        );
        assert!(
            matched[0].contains(expected_left) && matched[0].contains(expected_right),
            "{path} ({rule_id}): {}",
            matched[0]
        );
    }
}

#[test]
fn cross_language_corpus_never_compares_functions_across_languages() {
    let mut config = similarity_only_config();
    config.similarity.function_threshold = 0.60;
    let analyzer = Analyzer::default();

    let mut files = Vec::new();
    for (path, src, _, _) in polyglot_cases() {
        let Some(frontend) = analyzer.languages().frontend_for_path(Path::new(path)) else {
            panic!("missing frontend for {path}");
        };
        files.push(SourceFile::new(
            PathBuf::from(path),
            PathBuf::from(path),
            frontend.grammar(),
            src,
        ));
    }

    let report = analyzer.check_sources(&SourceTree::from_files(".", files), &config);
    let dup_diags: Vec<&Diagnostic> = report
        .diagnostics()
        .iter()
        .filter(|d| d.rule_id() == "duplicate-function")
        .collect();

    assert!(
        dup_diags.len() >= 3,
        "expected same-language duplicates across Rust, TS, and Swift: {dup_diags:?}"
    );
    let mut seen_exts = HashSet::new();
    for diag in &dup_diags {
        assert!(
            !diag.related().is_empty(),
            "duplicate-function diagnostic must attach related peer location: {}",
            diag.message()
        );
        let left_ext = diag.span().file().extension().and_then(|e| e.to_str());
        let right_ext = diag.related()[0]
            .span()
            .file()
            .extension()
            .and_then(|e| e.to_str());
        assert!(
            left_ext.is_some(),
            "missing extension on {}",
            diag.span().file().display()
        );
        assert_eq!(
            left_ext,
            right_ext,
            "cross-language duplicate must never be reported: {} vs {} ({})",
            diag.span().file().display(),
            diag.related()[0].span().file().display(),
            diag.message()
        );
        if let Some(ext) = left_ext {
            seen_exts.insert(ext.to_string());
        }
    }
    for required in ["rs", "ts", "swift"] {
        assert!(
            seen_exts.contains(required),
            "expected at least one same-language duplicate for `.{required}`, saw {seen_exts:?}"
        );
    }
}

#[test]
fn enum_cases_are_not_conflated_across_languages() {
    let config = similarity_only_config();
    let cases = [
        (
            "src/status.rs",
            "classify_active_first",
            "classify_active_clone",
            r"enum Status { Active, Inactive }
enum Mode { Fast, Slow }
fn classify_active_first(status: Status, mode: Mode, limit: i32) -> i32 {
    let base = match status { Status::Active => 100, Status::Inactive => 10 };
    let bonus = match mode { Mode::Fast => base + limit, Mode::Slow => base - limit };
    if bonus > 0 { bonus * 2 } else { 0 }
}
fn classify_active_clone(state: Status, speed: Mode, cap: i32) -> i32 {
    let seed = match state { Status::Active => 100, Status::Inactive => 10 };
    let extra = match speed { Mode::Fast => seed + cap, Mode::Slow => seed - cap };
    if extra > 0 { extra * 2 } else { 0 }
}
fn classify_inactive_first(status: Status, mode: Mode, limit: i32) -> i32 {
    let base = match status { Status::Inactive => 100, Status::Active => 10 };
    let bonus = match mode { Mode::Slow => base + limit, Mode::Fast => base - limit };
    if bonus > 0 { bonus * 2 } else { 0 }
}",
        ),
        (
            "src/status.ts",
            "classifyActiveFirst",
            "classifyActiveClone",
            r"enum Status { Active, Inactive }
enum Mode { Fast, Slow }
export function classifyActiveFirst(status: Status, mode: Mode, limit: number): number {
    if (status === Status.Active) { return mode === Mode.Fast ? limit + 100 : limit - 10; }
    if (status === Status.Inactive) { return mode === Mode.Slow ? limit * 2 : 0; }
    return 0;
}
export function classifyActiveClone(state: Status, speed: Mode, cap: number): number {
    if (state === Status.Active) { return speed === Mode.Fast ? cap + 100 : cap - 10; }
    if (state === Status.Inactive) { return speed === Mode.Slow ? cap * 2 : 0; }
    return 0;
}
export function classifyInactiveFirst(status: Status, mode: Mode, limit: number): number {
    if (status === Status.Inactive) { return mode === Mode.Slow ? limit + 100 : limit - 10; }
    if (status === Status.Active) { return mode === Mode.Fast ? limit * 2 : 0; }
    return 0;
}",
        ),
        (
            "Sources/Status.swift",
            "classifyActiveFirst",
            "classifyActiveClone",
            r"enum Status { case active; case inactive }
enum Mode { case fast; case slow }
func classifyActiveFirst(_ status: Status, _ mode: Mode, limit: Int) -> Int {
    let base: Int; switch status { case .active: base = 100; case .inactive: base = 10 }
    let bonus: Int; switch mode { case .fast: bonus = base + limit; case .slow: bonus = base - limit }
    return bonus > 0 ? bonus * 2 : 0
}
func classifyActiveClone(_ state: Status, _ speed: Mode, cap: Int) -> Int {
    let seed: Int; switch state { case .active: seed = 100; case .inactive: seed = 10 }
    let extra: Int; switch speed { case .fast: extra = seed + cap; case .slow: extra = seed - cap }
    return extra > 0 ? extra * 2 : 0
}
func classifyInactiveFirst(_ status: Status, _ mode: Mode, limit: Int) -> Int {
    let base: Int; switch status { case .inactive: base = 100; case .active: base = 10 }
    let bonus: Int; switch mode { case .slow: bonus = base + limit; case .fast: bonus = base - limit }
    return bonus > 0 ? bonus * 2 : 0
}",
        ),
    ];

    let mut dup_msgs = Vec::new();
    for (path, expected_left, expected_right, src) in cases {
        let report = check_single_file(path, src, &config);
        dup_msgs.clear();
        for diag in report.diagnostics() {
            if diag.rule_id() == "duplicate-function" {
                dup_msgs.push(diag.message().to_string());
            }
        }
        assert_eq!(
            dup_msgs.len(),
            1,
            "{path}: expected only the true renamed clone to match, got {dup_msgs:?}"
        );
        assert!(
            dup_msgs[0].contains(expected_left) && dup_msgs[0].contains(expected_right),
            "{path}: unexpected duplicate pair: {}",
            dup_msgs[0]
        );
    }
}

#[test]
fn language_dialect_edge_cases_match_only_true_clones() {
    let config = similarity_only_config();
    let cases = [
        (
            "overlapping-tests",
            "tests/order_assertions.rs",
            "checks_base",
            "checks_base_clone",
            r"
#[test]
fn checks_base() {
    let order = make_order(100);
    assert_eq!(order.status(), Status::Active);
    assert!(matches!(order.mode(), Mode::Fast));
    assert_eq!(order.items().collect::<Vec<_>>(), expected);
    assert_eq!(order.badge(), Badge::Gold);
}

#[test]
fn checks_base_clone() {
    let record = make_order(250);
    assert_eq!(record.status(), Status::Active);
    assert!(matches!(record.mode(), Mode::Fast));
    assert_eq!(record.items().collect::<Vec<_>>(), result);
    assert_eq!(record.badge(), Badge::Gold);
}

#[test]
fn checks_different_methods_only() {
    let order = make_order(100);
    assert_eq!(order.payment(), Status::Active);
    assert!(matches!(order.channel(), Mode::Fast));
    assert_eq!(order.entries().into_vec::<Vec<_>>(), expected);
    assert_eq!(order.coupon(), Badge::Gold);
}

#[test]
fn checks_different_variants_only() {
    let order = make_order(100);
    assert_eq!(order.status(), Status::Inactive);
    assert!(matches!(order.mode(), Mode::Slow));
    assert_eq!(order.items().collect::<Vec<_>>(), expected);
    assert_eq!(order.badge(), Badge::Silver);
}
",
        ),
        (
            "duplicate-function",
            "src/queries.ts",
            "queryUser",
            "queryAccount",
            r"
export function queryUser(id: number, active: boolean): string {
    if (/* keep condition */ active && id > 0) {
        const cleaned = (/* keep expr */ id + 1) as number;
        const rows = sql`select * from users where id = ${cleaned}`;
        return formatRows(rows, cleaned);
    }
    return fallbackValue(id);
}

export function queryAccount(userId: number, enabled: boolean): string {
    if (enabled && userId > 0) {
        const nextId = userId + 1;
        const result = sql`select * from users where id = ${nextId}`;
        return formatRows(result, nextId);
    }
    return fallbackValue(userId);
}

export function renderHtml(id: number, active: boolean): string {
    if (/* different cond */ !active || id < 0) {
        const cleaned = (/* different expr */ id * 3) as number;
        const markup = html`<div>${cleaned}</div>`;
        return formatRows(markup, cleaned);
    }
    return fallbackValue(id);
}
",
        ),
    ];

    let mut hits = Vec::new();
    for (rule_id, path, expected_left, expected_right, src) in cases {
        let report = check_single_file(path, src, &config);
        hits.clear();
        for diag in report.diagnostics() {
            if diag.rule_id() == rule_id {
                hits.push(diag.message().to_string());
            }
        }
        assert_eq!(
            hits.len(),
            1,
            "{path}: expected only ({expected_left}, {expected_right}) for `{rule_id}`, got {hits:?}"
        );
        assert!(
            hits[0].contains(expected_left) && hits[0].contains(expected_right),
            "{path}: unexpected `{rule_id}` pair: {}",
            hits[0]
        );
    }
}

#[test]
fn rust_macro_nested_turbofish_and_primitive_types_are_pinned() {
    let src = r"
fn collect_macro(x: &[i32], y: Vec<i32>) {
    assert_eq!(x.iter().collect::<Vec<_>>(), y);
    assert_eq!(foo::<<T as Tr>::X>(), y);
    assert_eq!(mod_a::lower_block { x: 1 }, y);
}

fn into_vec_macro(x: &[i32], y: Vec<i32>) {
    assert_eq!(x.iter().into_vec::<Vec<_>>(), y);
    assert_eq!(foo::<<T as Tr>::X>(), y);
    assert_eq!(mod_a::lower_block { x: 1 }, y);
}

fn cast_u8(x: i32) {
    assert_eq!(x as u8, 3);
}

fn cast_u64(x: i32) {
    assert_eq!(x as u64, 3);
}
";
    let tree = SyntaxTree::parse(
        "tests/macro_pin.rs",
        "tests/macro_pin.rs",
        rust_language_spec(),
        src,
    );
    let ast = RustFrontend.extract(&tree, Path::new("."));
    let get_fn = |name: &str| {
        ast.functions
            .iter()
            .find(|f| f.name == name)
            .unwrap_or_else(|| panic!("missing function `{name}`"))
    };
    let has_exact = |func_name: &str, token: &str| {
        get_fn(func_name)
            .body_tree
            .nodes()
            .iter()
            .any(|n| matches!(n.label(), NodeLabel::Exact(t) if t == token))
    };

    let collect_fn = get_fn("collect_macro");
    assert!(
        collect_fn
            .calls
            .iter()
            .any(|c| c.kind == CallKind::Method && c.callee == "collect"),
        "nested `>>` turbofish must record `collect` as a Method call: {:?}",
        collect_fn.calls
    );
    assert!(
        collect_fn
            .calls
            .iter()
            .any(|c| c.kind == CallKind::Free && c.callee == "foo"),
        "opening `::<<` turbofish must record `foo` as a Free call: {:?}",
        collect_fn.calls
    );
    assert!(
        collect_fn.field_inits.is_empty(),
        "lowercase scoped path followed by `{{}}` must not record struct field_inits: {:?}",
        collect_fn.field_inits
    );
    assert!(has_exact("collect_macro", "collect"));
    assert!(has_exact("into_vec_macro", "into_vec"));
    assert!(!has_exact("into_vec_macro", "collect"));
    assert!(has_exact("cast_u8", "u8"));
    assert!(has_exact("cast_u64", "u64"));
    assert!(!has_exact("cast_u64", "u8"));
}
