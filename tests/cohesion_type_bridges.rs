mod support;

use rqcode::config::Config;
use std::error::Error;
use support::{analyze_project, assert_no_rule, cohesion_only_config, diags_for};

fn tc_config(extra: &str) -> Result<Config, Box<dyn Error>> {
    let toml = format!(
        "[similarity]\nenabled = false\n[test_coupling]\nenabled = false\n[ast_grep]\nenabled = false\nbuiltin_rules = false\n[cohesion.type_cohesion]\n{extra}\n"
    );
    Ok(Config::from_toml(&toml)?)
}

#[test]
fn low_cohesion_type_connects_via_stateless_calls_and_enforces_receiver_identity()
-> Result<(), Box<dyn Error>> {
    let no_lex = tc_config("max_conceptual_similarity = 1.01")?;
    let make_src = |call_in_cluster_b: &str| {
        format!(
            r#"
pub struct CallConnected {{ smtp_host: String, smtp_port: u16, invoice_cents: u64, ledger_tax_bps: u64 }}
impl CallConnected {{
    pub fn scale(&self, v: u64) -> u64 {{ v + 1 }}
    pub fn format_smtp(&self) -> String {{ format!("{{}}:{{}}", self.smtp_host, self.smtp_port) }}
    pub fn check_port(&self) -> bool {{ self.scale(u64::from(self.smtp_port)) > 0 }}
    pub fn update_smtp(&mut self, port: u16) {{ self.smtp_port += port; }}
    pub fn invoice_tax(&self) -> u64 {{ {call_in_cluster_b} }}
    pub fn invoice_total(&self) -> u64 {{ self.invoice_cents + self.ledger_tax_bps }}
    pub fn adjust_invoice(&mut self, d: u64) {{ self.invoice_cents += d; }}
}}
"#
        )
    };
    assert_no_rule(
        &analyze_project(
            &[(
                "src/c1.rs",
                &make_src("self.scale((self.invoice_cents * self.ledger_tax_bps) / 10_000)"),
            )],
            &no_lex,
        )?,
        "low-cohesion-type",
    );
    assert_eq!(
        diags_for(
            &analyze_project(
                &[(
                    "src/c2.rs",
                    &make_src("(self.invoice_cents * self.ledger_tax_bps) / 10_000")
                )],
                &no_lex
            )?,
            "low-cohesion-type"
        )
        .len(),
        1
    );

    // Non-self calls (`self.smtp_host.len()`, `other.len()`, `Self::len(other)`), trait-impl call (`self.via_trait()`),
    // non-self field access (`other.smtp_port`), and sibling type `Shadow` must NOT bridge `Split`.
    let non_self_and_sibling = r"
pub trait Ext { fn via_trait(&self) -> u64; }
pub struct Shadow { smtp_port: u16, invoice_cents: u64 }
impl Shadow { pub fn bridge_shadow(&self) -> u64 { u64::from(self.smtp_port) + self.invoice_cents } }
pub struct Split { smtp_host: String, smtp_port: u16, invoice_cents: u64, ledger_tax_bps: u64 }
impl Ext for Split { fn via_trait(&self) -> u64 { u64::from(self.smtp_port) + self.invoice_cents } }
impl Split {
    pub fn format_smtp(&self) -> usize { self.smtp_host.len() + usize::from(self.smtp_port) + self.via_trait() as usize }
    pub fn check_port(&self, other: &Self) -> bool { let _ = (other.len(), Self::len(other)); self.smtp_port == 465 }
    pub fn update_smtp(&mut self, port: u16) { self.smtp_port += port; }
    pub fn len(&self) -> u64 { self.invoice_cents + self.ledger_tax_bps + self.via_trait() }
    pub fn compare_other(&self, other: &Self) -> u64 { self.invoice_cents + u64::from(other.smtp_port) }
    pub fn adjust_invoice(&mut self, d: u64) { self.invoice_cents += d; }
}
";
    let diags = analyze_project(&[("src/id.rs", non_self_and_sibling)], &no_lex)?;
    assert_eq!(
        diags_for(&diags, "low-cohesion-type").len(),
        1,
        "non-self/trait/sibling must not bridge: {diags:?}"
    );
    Ok(())
}

#[test]
fn low_cohesion_type_computes_multihop_and_exact_cyclic_transitive_closure()
-> Result<(), Box<dyn Error>> {
    // 2-hop closure: invoice_outer -> invoice_mid -> invoice_base{invoice_cents, ledger_tax_bps}.
    let two_hop = r#"
pub struct TwoHop { smtp_host: String, smtp_port: u16, invoice_cents: u64, ledger_tax_bps: u64 }
impl TwoHop {
    pub fn smtp_endpoint(&self) -> String { format!("{}:{}", self.smtp_host, self.smtp_port) }
    pub fn smtp_tls(&self) -> bool { self.smtp_port == 465 }
    pub fn invoice_base(&self) -> u64 { self.invoice_cents + self.ledger_tax_bps }
    pub fn invoice_mid(&self) -> u64 { self.invoice_base() + 1 }
    pub fn invoice_outer(&self) -> u64 { self.invoice_mid() + 2 }
}
"#;
    let five_field_cfg =
        tc_config("max_conceptual_similarity = 1.01\nmin_field_using_methods = 5")?;
    assert_eq!(
        diags_for(
            &analyze_project(&[("src/hop.rs", two_hop)], &five_field_cfg)?,
            "low-cohesion-type"
        )
        .len(),
        1
    );

    // Self & mutual recursion: `even_invoice` and `odd_invoice` must BOTH converge to {invoice_cents, ledger_tax_bps}
    // so `odd_invoice` also shares `invoice_cents` with `adjust_invoice` (exact TCC = 6/15 = 0.40, NOT 5/15 = 0.33).
    let cyclic = r#"
pub struct CyclicCalls { smtp_host: String, smtp_port: u16, invoice_cents: u64, ledger_tax_bps: u64 }
impl CyclicCalls {
    pub fn ping_smtp(&self, n: u64) -> String {
        if n > 0 { self.ping_smtp(n - 1) } else { format!("{}:{}", self.smtp_host, self.smtp_port) }
    }
    pub fn check_tls(&self) -> bool { self.smtp_port == 465 }
    pub fn bump_port(&mut self, p: u16) { self.smtp_port += p; }
    pub fn even_invoice(&self, n: u64) -> u64 { if n == 0 { self.invoice_cents } else { self.odd_invoice(n - 1) } }
    pub fn odd_invoice(&self, n: u64) -> u64 { if n == 0 { self.ledger_tax_bps } else { self.even_invoice(n - 1) } }
    pub fn adjust_invoice(&mut self, d: u64) { self.invoice_cents += d; }
}
"#;
    let default_hits = analyze_project(&[("src/cyc.rs", cyclic)], &cohesion_only_config())?;
    let hits = diags_for(&default_hits, "low-cohesion-type");
    assert_eq!(hits.len(), 1);
    assert!(
        hits[0].message().contains("TCC = 0.40"),
        "{}",
        hits[0].message()
    );
    assert_no_rule(
        &analyze_project(&[("src/cyc.rs", cyclic)], &tc_config("max_tcc = 0.40")?)?,
        "low-cohesion-type",
    );
    Ok(())
}

#[test]
fn low_cohesion_type_propagates_fields_through_rust_ts_and_swift_accessors()
-> Result<(), Box<dyn Error>> {
    let cfg = tc_config("max_conceptual_similarity = 1.01")?;

    let make_rust = |bridge_expr: &str| {
        format!(
            r#"
pub struct Account {{ ledger_cents: u64, tax_bps: u64, smtp_host: String, smtp_port: u16 }}
impl Account {{
    pub fn balance(&self) -> u64 {{ self.ledger_cents }}
    pub fn deposit(&mut self, v: u64) {{ self.ledger_cents += v; }}
    pub fn compute_tax(&self) -> u64 {{ (self.ledger_cents * self.tax_bps) / 10_000 }}
    pub fn format_endpoint(&self) -> String {{ format!("{{}}:{{}}:{{}}", self.smtp_host, self.smtp_port, {bridge_expr}) }}
    pub fn check_tls(&self) -> bool {{ self.smtp_port == 465 }}
    pub fn bump_port(&mut self, p: u16) {{ self.smtp_port += p; }}
}}
"#
        )
    };
    assert_no_rule(
        &analyze_project(&[("src/acc.rs", &make_rust("self.balance()"))], &cfg)?,
        "low-cohesion-type",
    );
    assert_eq!(
        diags_for(
            &analyze_project(&[("src/acc.rs", &make_rust("0"))], &cfg)?,
            "low-cohesion-type"
        )
        .len(),
        1
    );

    let make_ts = |bridge_stmt: &str| {
        format!(
            r#"
export class Wallet {{
    private cents: number = 0; private taxBps: number = 0; private smtpHost: string = ""; private smtpPort: number = 587;
    get balance(): number {{ return this.cents; }}
    set balance(v: number) {{ this.cents = v; }}
    addFunds(v: number): void {{ this.cents += v; }}
    computeTax(): number {{ return (this.cents * this.taxBps) / 10000; }}
    formatEndpoint(): string {{ return this.smtpHost + ":" + this.smtpPort; }}
    isTls(): boolean {{ return this.smtpPort === 465; }}
    bumpPort(p: number): void {{ {bridge_stmt}; this.smtpPort += p; }}
}}
"#
        )
    };
    assert_no_rule(
        &analyze_project(&[("src/w1.ts", &make_ts("const b = this.balance"))], &cfg)?,
        "low-cohesion-type",
    );
    assert_no_rule(
        &analyze_project(&[("src/w2.ts", &make_ts("this.balance = p"))], &cfg)?,
        "low-cohesion-type",
    );
    assert_eq!(
        diags_for(
            &analyze_project(&[("src/w3.ts", &make_ts("const b = p"))], &cfg)?,
            "low-cohesion-type"
        )
        .len(),
        1
    );

    let make_swift_computed = |bridge_expr: &str| {
        format!(
            r#"
public final class SwiftWallet {{
    private var cents: Int = 0; private var taxBps: Int = 0; private var smtpHost: String = ""; private var smtpPort: Int = 587
    public var balance: Int {{ return self.cents }}
    public func addFunds(v: Int) {{ self.cents += v }}
    public func computeTax() -> Int {{ return (self.cents * self.taxBps) / 10000 }}
    public func formatEndpoint() -> String {{ return "\(self.smtpHost):\(self.smtpPort):\({bridge_expr})" }}
    public func isTls() -> Bool {{ return self.smtpPort == 465 && {bridge_expr} > 0 }}
    public func bumpPort(p: Int) {{ self.smtpPort += p }}
}}
"#
        )
    };
    assert_no_rule(
        &analyze_project(
            &[(
                "Sources/App/SW1.swift",
                &make_swift_computed("self.balance"),
            )],
            &cfg,
        )?,
        "low-cohesion-type",
    );
    assert_eq!(
        diags_for(
            &analyze_project(
                &[("Sources/App/SW2.swift", &make_swift_computed("1"))],
                &cfg
            )?,
            "low-cohesion-type"
        )
        .len(),
        1
    );
    Ok(())
}

#[test]
fn low_cohesion_type_propagates_fields_through_swift_overloads() -> Result<(), Box<dyn Error>> {
    let cfg = tc_config("max_conceptual_similarity = 1.01")?;
    let make_swift = |tax_call: &str, tls_call: &str| {
        format!(
            r#"
public final class Overloaded {{
    private var ledgerCents: Int = 0; private var taxBps: Int = 0; private var smtpHost: String = ""; private var smtpPort: Int = 587
    public func touch(a: Int) {{ self.ledgerCents += a }}
    public func touch(b: String) {{ self.smtpPort += b.count }}
    public func taxAmount() -> Int {{ {tax_call}; return (self.ledgerCents * self.taxBps) / 10000 }}
    public func formatEndpoint() -> String {{ return "\(self.smtpHost):\(self.smtpPort)" }}
    public func isTls() -> Bool {{ {tls_call}; return self.smtpPort == 465 }}
}}
"#
        )
    };
    assert_no_rule(
        &analyze_project(
            &[(
                "Sources/App/O1.swift",
                &make_swift("let _ = 0", "self.touch(a: 1)"),
            )],
            &cfg,
        )?,
        "low-cohesion-type",
    );
    assert_no_rule(
        &analyze_project(
            &[(
                "Sources/App/O2.swift",
                &make_swift("self.touch(b: \"x\")", "let _ = 0"),
            )],
            &cfg,
        )?,
        "low-cohesion-type",
    );
    assert_eq!(
        diags_for(
            &analyze_project(
                &[(
                    "Sources/App/O3.swift",
                    &make_swift("let _ = 0", "let _ = 0")
                )],
                &cfg
            )?,
            "low-cohesion-type"
        )
        .len(),
        1
    );
    Ok(())
}

#[test]
fn low_cohesion_type_isolates_swift_observer_field_seed_and_write_edge()
-> Result<(), Box<dyn Error>> {
    let cfg = tc_config("max_conceptual_similarity = 1.01")?;

    // Isolation 1 (property-observer field seed alone): ZERO internal writers of `ledgerCents`.
    let make_seed = |observer: &str| {
        format!(
            r#"
public final class SeedOnly {{
    private var ledgerCents: Int = 0{observer}
    private var taxBps: Int = 0; private var smtpHost: String = ""; private var smtpPort: Int = 587
    public func netCents() -> Int {{ return self.ledgerCents + 1 }}
    public func grossCents() -> Int {{ return self.ledgerCents * 2 }}
    public func taxAmount() -> Int {{ return (self.ledgerCents * self.taxBps) / 10000 }}
    public func formatEndpoint() -> String {{ return "\(self.smtpHost):\(self.smtpPort)" }}
    public func isTls() -> Bool {{ return self.smtpPort == 465 }}
    public func bumpPort(p: Int) {{ self.smtpPort += p }}
}}
"#
        )
    };
    let seed_obs = " {\n        didSet { self.smtpPort += 1 }\n    }";
    assert_no_rule(
        &analyze_project(&[("Sources/App/Seed1.swift", &make_seed(seed_obs))], &cfg)?,
        "low-cohesion-type",
    );
    assert_eq!(
        diags_for(
            &analyze_project(&[("Sources/App/Seed2.swift", &make_seed(""))], &cfg)?,
            "low-cohesion-type"
        )
        .len(),
        1
    );

    // Isolation 2 (write-to-observer edge alone): trivial `didSet { self.smtpPort }` is excluded from M_elig,
    // so ONLY the write-to-observer edge from `deposit` propagates `smtpPort` into F*(deposit).
    let make_write = |deposit_body: &str| {
        format!(
            r#"
public final class WriteOnly {{
    private var ledgerCents: Int = 0 {{
        didSet {{ self.smtpPort }}
    }}
    private var taxBps: Int = 0; private var smtpHost: String = ""; private var smtpPort: Int = 587
    public func deposit(v: Int) -> Int {{ {deposit_body} }}
    public func grossCents() -> Int {{ return self.ledgerCents * 2 }}
    public func taxAmount() -> Int {{ return (self.ledgerCents * self.taxBps) / 10000 }}
    public func formatEndpoint() -> String {{ return "\(self.smtpHost):\(self.smtpPort)" }}
    public func isTls() -> Bool {{ return self.smtpPort == 465 }}
    public func bumpPort(p: Int) {{ self.smtpPort += p }}
}}
"#
        )
    };
    assert_no_rule(
        &analyze_project(
            &[(
                "Sources/App/W1.swift",
                &make_write("self.ledgerCents += v; return self.ledgerCents"),
            )],
            &cfg,
        )?,
        "low-cohesion-type",
    );
    assert_eq!(
        diags_for(
            &analyze_project(
                &[(
                    "Sources/App/W2.swift",
                    &make_write("return self.ledgerCents + v")
                )],
                &cfg
            )?,
            "low-cohesion-type"
        )
        .len(),
        1
    );
    Ok(())
}
