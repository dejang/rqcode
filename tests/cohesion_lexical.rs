use rqcode::ast::SyntaxTree;
use rqcode::ast::codebase::Codebase;
use rqcode::config::{CohesionConfig, Config, TypeCohesionConfig};
use rqcode::diagnostic::Severity;
use rqcode::lang::LanguageFrontend;
use rqcode::lang::rust::{RustFrontend, rust_language_spec};
use rqcode::lang::swift::{SwiftFrontend, swift_language_spec};
use rqcode::lang::typescript::{TypeScriptFrontend, typescript_language_spec};
use rqcode::rules::cohesion::lexical::{
    LexicalCorpus, LexicalParams, SparseTfIdf, cosine_similarity, tokenize_identifier,
};
use rqcode::rules::cohesion::type_cohesion::is_trivial_accessor;
use std::collections::HashMap;
use std::error::Error;
use std::ffi::OsStr;
use std::path::Path;

fn build_codebase(files: &[(&str, &str)]) -> Codebase {
    let mut trees = Vec::with_capacity(files.len());
    let mut asts = Vec::with_capacity(files.len());
    for &(path, src) in files {
        let (tree, ast) = match Path::new(path).extension().and_then(OsStr::to_str) {
            Some("ts") => {
                let tree = SyntaxTree::parse(path, path, typescript_language_spec(), src);
                let ast = TypeScriptFrontend.extract(&tree, Path::new("."));
                (tree, ast)
            }
            Some("swift") => {
                let tree = SyntaxTree::parse(path, path, swift_language_spec(), src);
                let ast = SwiftFrontend::new().extract(&tree, Path::new("."));
                (tree, ast)
            }
            _ => {
                let tree = SyntaxTree::parse(path, path, rust_language_spec(), src);
                let ast = RustFrontend.extract(&tree, Path::new("."));
                (tree, ast)
            }
        };
        trees.push(tree);
        asts.push(ast);
    }
    Codebase::build(trees, asts)
}

#[test]
fn tokenize_identifier_splits_cases_acronyms_punctuation_and_all_default_stopwords() {
    let defaults = LexicalParams::default();
    assert_eq!(defaults.min_token_len, 3);
    assert_eq!(
        tokenize_identifier("calculateOrderTax", &defaults),
        vec!["calculate", "order", "tax"]
    );
    assert_eq!(
        tokenize_identifier("calculate_order_tax", &defaults),
        vec!["calculate", "order", "tax"]
    );
    assert_eq!(
        tokenize_identifier("parseHTTPHeader", &defaults),
        vec!["parse", "http", "header"]
    );
    assert_eq!(
        tokenize_identifier("HTMLParser", &defaults),
        vec!["html", "parser"]
    );
    assert_eq!(
        tokenize_identifier("models::Order", &defaults),
        vec!["models", "order"]
    );
    assert_eq!(tokenize_identifier("#port", &defaults), vec!["port"]);
    assert_eq!(tokenize_identifier("_port", &defaults), vec!["port"]);
    assert_eq!(
        tokenize_identifier("SCREAMING_SNAKE", &defaults),
        vec!["screaming", "snake"]
    );
    assert_eq!(tokenize_identifier("userID", &defaults), vec!["user"]);
    assert_eq!(
        tokenize_identifier("sha256Hash", &defaults),
        vec!["sha256", "hash"]
    );

    let all_stopwords = [
        "self",
        "this",
        "get",
        "set",
        "new",
        "from",
        "into",
        "with",
        "val",
        "value",
        "item",
        "data",
        "list",
        "vec",
        "map",
        "str",
        "string",
        "int",
        "bool",
        "void",
        "async",
        "await",
        "return",
        "result",
        "option",
        "error",
        "err",
        "true",
        "false",
        "null",
        "undefined",
        "len",
        "push",
        "pop",
        "iter",
        "clone",
        "default",
        "index",
        "count",
        "size",
        "key",
        "name",
        "type",
        "kind",
        "span",
        "node",
    ];
    for sw in all_stopwords {
        assert_eq!(
            tokenize_identifier(sw, &defaults),
            Vec::<String>::new(),
            "stopword `{sw}`"
        );
    }

    let custom = LexicalParams {
        min_token_len: 2,
        stopwords: vec![String::from("order")],
    };
    assert_eq!(
        tokenize_identifier("getOrderId", &custom),
        vec!["get", "id"]
    );
}

#[test]
fn lexical_corpus_computes_exact_smoothed_idf_sublinear_tf_and_excludes_test_files() {
    let lib_src = r"
pub fn check_smtp_relay(smtp_host: &str) -> usize { smtp_host.len() }
pub fn check_smtp_envelope(smtp_host: &str) -> usize { smtp_host.len() }
pub fn check_invoice_ledger(invoice_code: &str) -> usize { invoice_code.len() }
pub fn check_invoice_audit(invoice_code: &str) -> usize { invoice_code.len() }

#[cfg(test)]
mod tests {
    #[test]
    fn test_smtp_relay_invoice_ledger() {
        let _ = (super::check_smtp_relay, super::check_invoice_ledger);
    }
}
";
    let test_file_src =
        "pub fn helper_smtp_relay_quasar(smtp_host: &str) -> usize { smtp_host.len() }\n";
    let codebase = build_codebase(&[("src/lib.rs", lib_src), ("tests/helper.rs", test_file_src)]);
    let corpus = LexicalCorpus::build(&codebase, LexicalParams::default());

    let idf_smtp = (5.0_f64 / 3.0).ln() + 1.0;
    let w_smtp = (1.0 + 3.0_f64.ln()) * idf_smtp;
    let expected_cos = w_smtp / (w_smtp * w_smtp + 1.0).sqrt();

    let v_multi = corpus.vector_from_tokens(&["smtp", "smtp", "smtp", "check"]);
    let v_smtp = corpus.vector_from_tokens(&["smtp"]);
    let v_invoice = corpus.vector_from_tokens(&["invoice", "ledger"]);
    let v_empty = SparseTfIdf::default();
    assert!((cosine_similarity(&v_multi, &v_smtp) - expected_cos).abs() < 1e-9);
    assert!((cosine_similarity(&v_smtp, &v_smtp) - 1.0).abs() < 1e-9);
    assert!((cosine_similarity(&v_smtp, &v_invoice) - 0.0).abs() < 1e-9);
    assert!((cosine_similarity(&v_smtp, &v_empty) - 0.0).abs() < 1e-9);

    let idf_unseen = 5.0_f64.ln() + 1.0;
    let expected_unseen = idf_unseen / (idf_unseen * idf_unseen + 1.0).sqrt();
    let v_unseen_mix = corpus.vector_from_tokens(&["quasar", "check"]);
    let v_unseen = corpus.vector_from_tokens(&["quasar"]);
    assert!((cosine_similarity(&v_unseen_mix, &v_unseen) - expected_unseen).abs() < 1e-9);
}

#[test]
fn lexical_corpus_isolates_all_five_method_sources_foreign_fields_and_sublinear_tf()
-> Result<(), Box<dyn Error>> {
    let src = r"
pub struct Foreign { pub omega_foreign: u8 }
pub struct Holder { delta_field: u8 }
impl Holder {
    pub fn alpha_verb(&self, _beta_param: models::GammaPayload, other: &Foreign, ext: UnknownExt) -> u8 {
        let theta_local = self.delta_field + other.omega_foreign + ext.zeta_unresolved;
        other.epsilon_callee(theta_local)
    }
    pub fn delta_run(&self, _delta_in: u8, _solo_in: u8) -> u8 { self.delta_field }
    pub fn filler_one(&self) -> u8 { 1 }
}
";
    let codebase = build_codebase(&[("src/sources.rs", src)]);
    let corpus = LexicalCorpus::build(&codebase, LexicalParams::default());
    let fns: HashMap<&str, _> = codebase
        .functions()
        .iter()
        .map(|f| (f.decl.name.as_str(), f))
        .collect();
    let v_alpha = corpus.method_vector(*fns.get("alpha_verb").ok_or("missing alpha_verb")?);

    let idf_df1 = (4.0_f64 / 2.0).ln() + 1.0;
    let idf_df0 = 4.0_f64.ln() + 1.0;
    let expected_df1_vs_df0 = idf_df1 / (idf_df1 * idf_df1 + idf_df0 * idf_df0).sqrt();

    for source_token in ["alpha", "beta", "gamma", "omega", "epsilon", "theta"] {
        let probe = corpus.vector_from_tokens(&[source_token]);
        assert!(
            cosine_similarity(&v_alpha, &probe) > 0.0,
            "missing `{source_token}` in T(m)"
        );
        let pair = corpus.vector_from_tokens(&[source_token, "unseen"]);
        assert!(
            (cosine_similarity(&pair, &probe) - expected_df1_vs_df0).abs() < 1e-9,
            "token `{source_token}` must have df = 1"
        );
    }
    assert!(cosine_similarity(&v_alpha, &corpus.vector_from_tokens(&["delta"])) > 0.0);
    assert!(
        (cosine_similarity(&v_alpha, &corpus.vector_from_tokens(&["zeta"])) - 0.0).abs() < 1e-9
    );

    let v_repeat = corpus.method_vector(*fns.get("delta_run").ok_or("missing delta_run")?);
    let idf2 = (4.0_f64 / 3.0).ln() + 1.0;
    let w_delta = (1.0 + 3.0_f64.ln()) * idf2;
    let norm = (w_delta * w_delta + idf2 * idf2 + 2.0 * idf_df1 * idf_df1).sqrt();
    let expected_tf3 = w_delta / norm;
    let actual_tf3 = cosine_similarity(&v_repeat, &corpus.vector_from_tokens(&["delta"]));
    assert!(
        (actual_tf3 - expected_tf3).abs() < 1e-9,
        "expected exact sublinear tf=3 cosine {expected_tf3}, got {actual_tf3}"
    );
    Ok(())
}

#[test]
fn trivial_accessor_accepts_polyglot_getters_setters_and_enforces_node_budget()
-> Result<(), Box<dyn Error>> {
    let max_nodes = TypeCohesionConfig::default().max_accessor_nodes;
    let rust_src = r"
pub struct Boxed { host: String, port: u16 }
impl Boxed {
    pub fn port(&self) -> u16 { self.port }
    pub fn host(&self) -> &str { &self.host }
    pub fn port_mut(&mut self) -> &mut u16 { &mut self.port }
    pub fn set_port(&mut self, p: u16) { self.port = p; }
}
";
    let rust_cb = build_codebase(&[("src/boxed.rs", rust_src)]);
    let boxed = &rust_cb.types()[0];
    let rust_fns: HashMap<&str, _> = rust_cb
        .functions()
        .iter()
        .map(|f| (f.decl.name.as_str(), f))
        .collect();
    for pos in ["port", "host", "port_mut", "set_port"] {
        let f = *rust_fns.get(pos).ok_or("missing positive fn")?;
        assert!(
            is_trivial_accessor(f, boxed, max_nodes),
            "{pos} must be trivial"
        );
    }
    let set_port = *rust_fns.get("set_port").ok_or("missing set_port")?;
    let exact_len = set_port.decl.body_tree.len();
    assert!(exact_len >= 2);
    assert!(is_trivial_accessor(set_port, boxed, exact_len));
    assert!(!is_trivial_accessor(set_port, boxed, exact_len - 1));
    assert!(!is_trivial_accessor(
        *rust_fns.get("port").ok_or("missing port")?,
        boxed,
        0
    ));

    // Pass each method's own `body_tree.len()` so the node budget never masks structural clauses
    // (such as `multiAssign`'s 2 `AstKind::Assign` nodes). Positive cases also pass default `max_nodes`.
    let ts_cb = build_codebase(&[(
        "src/box.ts",
        r"export class Box {
    private _port: number = 0;
    get port(): number { return this._port; }
    set port(v: number) { this._port = v; }
    get alias(): number { return this.port; }
    set multiAssign(v: number) { v = 1; this._port = v; }
    static peek(): number { return this._port; }
}
",
    )]);
    let ts_box = &ts_cb.types()[0];
    assert_eq!(ts_cb.functions().len(), 5);
    for f in ts_cb.functions() {
        let budget = if f.decl.name == "port" {
            max_nodes
        } else {
            f.decl.body_tree.len()
        };
        assert_eq!(
            is_trivial_accessor(f, ts_box, budget),
            f.decl.name == "port",
            "TS {} mismatch",
            f.decl.name
        );
    }

    let swift_cb = build_codebase(&[(
        "Sources/App/Box.swift",
        "public struct Box { private var port: Int = 0\n public var readPort: Int { return self.port }\n}\n",
    )]);
    let swift_box = &swift_cb.types()[0];
    let read_port = swift_cb
        .functions()
        .iter()
        .find(|f| f.decl.name == "readPort")
        .ok_or("missing readPort")?;
    assert!(is_trivial_accessor(read_port, swift_box, max_nodes));
    Ok(())
}

#[test]
fn trivial_accessor_rejects_disallowed_ast_kinds_non_self_and_non_inherent_methods()
-> Result<(), Box<dyn Error>> {
    let rust_src = r#"
pub struct Inner { port: u16 }
pub struct Wrap { inner: u16 }
pub trait HasPort { fn trait_port(&self) -> u16; }
pub struct Boxed { host: String, port: u16, code: u16, flag: bool, opt: Option<u16>, items: Vec<u16>, inner: Inner }
impl Boxed {
    pub fn port(&self) -> u16 { self.port }
    pub fn host_clone(&self) -> String { self.host.clone() }
    pub fn port_fmt(&self) -> String { format!("{}", self.port) }
    pub fn flag_if(&self) -> u16 { if self.flag { 1 } else { 0 } }
    pub fn port_match(&self) -> u16 { match self.port { _ => 1 } }
    pub fn port_loop(&self) -> u16 { loop { break self.port; } }
    pub fn port_try(&self) -> Option<u16> { self.opt?; None }
    pub fn port_closure(&self) -> impl Fn() -> u16 + '_ { || self.port }
    pub fn port_bin(&self) -> u16 { self.port + 1 }
    pub fn first_item(&self) -> u16 { self.items[0] }
    pub fn wrap_port(&self) -> Wrap { Wrap { inner: self.port } }
    pub fn bind_port(&self) -> u16 { let x = self.port; x }
    pub fn bump_port(&mut self) { self.port += 1; }
    pub fn add_port(&mut self, d: u16) { self.port += d; }
    pub fn double_assign(&mut self, mut p: u16) { p = 1; self.port = p; }
    pub fn pair(&self) -> (u16, u16) { (self.port, self.code) }
    pub fn chained(&self) -> u16 { self.inner.port }
    pub fn set_other(&self, other: &mut Self) { other.port = 0; }
    pub fn getter_with_param(&self, _x: u16) -> u16 { self.port }
    pub fn setter_no_param(&mut self) { self.port = 0; }
    pub fn setter_two_params(&mut self, a: u16, _b: u16) { self.port = a; }
}
impl HasPort for Boxed { fn trait_port(&self) -> u16 { self.port } }
#[cfg(test)]
mod tests {
    use super::Boxed;
    impl Boxed { fn test_port(&self) -> u16 { self.port } }
}
"#;
    let rust_cb = build_codebase(&[("src/neg.rs", rust_src)]);
    let boxed = rust_cb
        .types()
        .iter()
        .find(|t| t.decl.name == "Boxed")
        .ok_or("missing Boxed")?;
    let inner = rust_cb
        .types()
        .iter()
        .find(|t| t.decl.name == "Inner")
        .ok_or("missing Inner")?;
    let rust_fns: HashMap<&str, _> = rust_cb
        .functions()
        .iter()
        .map(|f| (f.decl.name.as_str(), f))
        .collect();

    let port_fn = *rust_fns.get("port").ok_or("missing port")?;
    assert!(!is_trivial_accessor(
        port_fn,
        inner,
        port_fn.decl.body_tree.len()
    ));

    // Pass `f.decl.body_tree.len()` for every negative case so node budget NEVER masks its clause.
    for neg in [
        "host_clone",
        "port_fmt",
        "flag_if",
        "port_match",
        "port_loop",
        "port_try",
        "port_closure",
        "port_bin",
        "first_item",
        "wrap_port",
        "bind_port",
        "bump_port",
        "add_port",
        "double_assign",
        "pair",
        "chained",
        "set_other",
        "getter_with_param",
        "setter_no_param",
        "setter_two_params",
        "trait_port",
        "test_port",
    ] {
        let f = *rust_fns.get(neg).ok_or("missing negative fn")?;
        assert!(
            !is_trivial_accessor(f, boxed, f.decl.body_tree.len()),
            "{neg} must NOT be trivial"
        );
    }
    Ok(())
}

#[test]
fn type_cohesion_config_defaults_match_spec() {
    let cohesion_cfg = CohesionConfig::default();
    assert_eq!(cohesion_cfg.severity, Severity::Error);
    assert_eq!(cohesion_cfg.advisory_severity, Severity::Warning);

    let tc = TypeCohesionConfig::default();
    assert!(tc.enabled);
    assert_eq!(tc.min_fields, 4);
    assert_eq!(tc.min_methods, 5);
    assert_eq!(tc.min_field_using_methods, 4);
    assert_eq!(tc.min_component_methods, 2);
    assert_eq!(tc.max_accessor_nodes, 10);
    assert!((tc.max_tcc - 0.50).abs() < 1e-9);
    assert!((tc.max_conceptual_similarity - 0.20).abs() < 1e-9);
}

#[test]
fn type_cohesion_config_toml_overrides_and_partial_defaults() -> Result<(), Box<dyn Error>> {
    let parsed = Config::from_toml(
        "[cohesion]\nadvisory_severity = \"error\"\n[cohesion.type_cohesion]\nenabled = false\nmin_fields = 5\nmin_methods = 6\nmin_field_using_methods = 5\nmin_component_methods = 3\nmax_accessor_nodes = 8\nmax_tcc = 0.40\nmax_conceptual_similarity = 0.25\n",
    )?;
    assert_eq!(parsed.cohesion.advisory_severity, Severity::Error);
    let ptc = &parsed.cohesion.type_cohesion;
    assert!(!ptc.enabled);
    assert_eq!(ptc.min_fields, 5);
    assert_eq!(ptc.min_methods, 6);
    assert_eq!(ptc.min_field_using_methods, 5);
    assert_eq!(ptc.min_component_methods, 3);
    assert_eq!(ptc.max_accessor_nodes, 8);
    assert!((ptc.max_tcc - 0.40).abs() < 1e-9);
    assert!((ptc.max_conceptual_similarity - 0.25).abs() < 1e-9);

    let partial = Config::from_toml("[cohesion.type_cohesion]\nmax_tcc = 0.35\n")?;
    assert!(partial.cohesion.type_cohesion.enabled);
    assert_eq!(partial.cohesion.type_cohesion.min_fields, 4);
    assert!((partial.cohesion.type_cohesion.max_tcc - 0.35).abs() < 1e-9);
    Ok(())
}
