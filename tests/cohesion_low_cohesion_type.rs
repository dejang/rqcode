mod support;

use rqcode::config::Config;
use rqcode::diagnostic::Severity;
use std::error::Error;
use support::{analyze_project, assert_no_rule, cohesion_only_config, diags_for};

fn type_cohesion_toml(cohesion_section: &str, tc_section: &str) -> Result<Config, Box<dyn Error>> {
    let toml = format!(
        "[similarity]\nenabled = false\n[test_coupling]\nenabled = false\n[ast_grep]\nenabled = false\nbuiltin_rules = false\n[cohesion]\n{cohesion_section}\n[cohesion.type_cohesion]\n{tc_section}\n"
    );
    Ok(Config::from_toml(&toml)?)
}

fn rust_billing_mailer(prefix: &str, extra_impl: &str) -> String {
    format!(
        r#"{prefix}pub struct BillingMailer {{
    smtp_host: String,
    smtp_port: u16,
    invoice_cents: u64,
    ledger_tax_bps: u64,
}}

impl BillingMailer {{
    pub fn format_smtp_endpoint(&self) -> String {{
        format!("{{}}:{{}}", self.smtp_host, self.smtp_port)
    }}
    pub fn smtp_uses_tls_port(&self) -> bool {{
        self.smtp_port == 465 || self.smtp_port == 587
    }}
    pub fn configure_smtp_relay(&mut self, host: String, port: u16) {{
        if !host.is_empty() {{
            self.smtp_host = host;
            self.smtp_port = port;
        }}
    }}
    pub fn compute_invoice_tax(&self) -> u64 {{
        (self.invoice_cents * self.ledger_tax_bps) / 10_000
    }}
    pub fn invoice_gross_total(&self) -> u64 {{
        self.invoice_cents + ((self.invoice_cents * self.ledger_tax_bps) / 10_000)
    }}
    pub fn post_ledger_adjustment(&mut self, delta: u64) {{
        if delta > 0 {{
            self.invoice_cents += delta;
        }}
    }}
}}
{extra_impl}"#
    )
}

#[test]
fn low_cohesion_type_flags_disjoint_responsibilities_across_rust_ts_and_swift()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();
    let rust_src = rust_billing_mailer("", "");

    let ts_src = r#"export class BillingMailer {
    private smtpHost: string = "";
    private smtpPort: number = 587;
    private invoiceCents: number = 0;
    private ledgerTaxBps: number = 0;

    formatSmtpEndpoint(): string { return this.smtpHost + ":" + this.smtpPort; }
    smtpUsesTlsPort(): boolean { return this.smtpPort === 465 || this.smtpPort === 587; }
    configureSmtpRelay(host: string, port: number): void {
        if (host.length > 0) { this.smtpHost = host; this.smtpPort = port; }
    }
    computeInvoiceTax(): number { return Math.floor((this.invoiceCents * this.ledgerTaxBps) / 10000); }
    invoiceGrossTotal(): number { return this.invoiceCents + Math.floor((this.invoiceCents * this.ledgerTaxBps) / 10000); }
    postLedgerAdjustment(delta: number): void { if (delta > 0) { this.invoiceCents += delta; } }
}
"#;

    let swift_src = r#"public final class BillingMailer {
    private var smtpHost: String = ""
    private var smtpPort: Int = 587
    private var invoiceCents: Int = 0
    private var ledgerTaxBps: Int = 0

    public func formatSmtpEndpoint() -> String { return "\(self.smtpHost):\(self.smtpPort)" }
    public func smtpUsesTlsPort() -> Bool { return self.smtpPort == 465 || self.smtpPort == 587 }
    public func configureSmtpRelay(host: String, port: Int) {
        if !host.isEmpty { self.smtpHost = host; self.smtpPort = port }
    }
    public func computeInvoiceTax() -> Int { return (self.invoiceCents * self.ledgerTaxBps) / 10000 }
    public func invoiceGrossTotal() -> Int { return self.invoiceCents + (self.invoiceCents * self.ledgerTaxBps) / 10000 }
    public func postLedgerAdjustment(delta: Int) { if delta > 0 { self.invoiceCents += delta } }
}
"#;

    for (label, path, src, c1_method, c1_field, c2_method, c2_field) in [
        (
            "rust",
            "src/billing.rs",
            rust_src.as_str(),
            "format_smtp_endpoint",
            "smtp_host",
            "compute_invoice_tax",
            "invoice_cents",
        ),
        (
            "typescript",
            "src/billing.ts",
            ts_src,
            "formatSmtpEndpoint",
            "smtpHost",
            "computeInvoiceTax",
            "invoiceCents",
        ),
        (
            "swift",
            "Sources/App/Billing.swift",
            swift_src,
            "formatSmtpEndpoint",
            "smtpHost",
            "computeInvoiceTax",
            "invoiceCents",
        ),
    ] {
        let diags = analyze_project(&[(path, src)], &cfg)?;
        let hits = diags_for(&diags, "low-cohesion-type");
        assert_eq!(hits.len(), 1, "{label}: expected 1 hit, got: {diags:?}");
        let hit = hits[0];
        assert_eq!(hit.severity(), Severity::Warning);
        assert!(hit.span().file().ends_with(path));
        assert_eq!(hit.span().start_line(), 1, "{label}: anchor at type decl");
        assert!(hit.message().contains("BillingMailer"));
        assert!(
            hit.message().contains("TCC = 0.40"),
            "{label}: {}",
            hit.message()
        );
        assert!(
            hit.message().contains("2 disjoint"),
            "{label}: {}",
            hit.message()
        );
        assert_eq!(hit.related().len(), 2, "{label}: {:?}", hit.related());
        assert!(
            hit.related()[0].label().contains(c1_method)
                && hit.related()[0].label().contains(c1_field)
        );
        assert!(
            hit.related()[1].label().contains(c2_method)
                && hit.related()[1].label().contains(c2_field)
        );
        let suggestion = hit.suggestion().unwrap_or("");
        assert!(
            suggestion.contains("BillingMailer")
                && suggestion.to_ascii_lowercase().contains("split")
        );
    }
    Ok(())
}

#[test]
fn low_cohesion_type_respects_advisory_severity_toggles_and_suppression()
-> Result<(), Box<dyn Error>> {
    let rust_src = rust_billing_mailer("", "");
    let files = &[("src/billing.rs", rust_src.as_str())];

    let promoted = analyze_project(
        files,
        &type_cohesion_toml("advisory_severity = \"error\"", "")?,
    )?;
    let promoted_hits = diags_for(&promoted, "low-cohesion-type");
    assert_eq!(promoted_hits.len(), 1);
    assert_eq!(promoted_hits[0].severity(), Severity::Error);

    assert_no_rule(
        &analyze_project(files, &type_cohesion_toml("", "enabled = false")?)?,
        "low-cohesion-type",
    );
    assert_no_rule(
        &analyze_project(files, &type_cohesion_toml("enabled = false", "")?)?,
        "low-cohesion-type",
    );

    let suppressed_src = rust_billing_mailer("// rqcode:allow(low-cohesion-type)\n", "");
    assert_no_rule(
        &analyze_project(
            &[("src/billing.rs", suppressed_src.as_str())],
            &cohesion_only_config(),
        )?,
        "low-cohesion-type",
    );
    Ok(())
}

#[test]
fn low_cohesion_type_ignores_glue_constructors_factories_interfaces_static_and_test_methods()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    let rust_with_glue = rust_billing_mailer(
        "",
        r#"
impl BillingMailer {
    pub fn new(smtp_host: String, smtp_port: u16, invoice_cents: u64, ledger_tax_bps: u64) -> Self {
        Self { smtp_host, smtp_port, invoice_cents, ledger_tax_bps }
    }
    pub fn duplicate(&self) -> Self {
        Self { smtp_host: self.smtp_host.clone(), smtp_port: self.smtp_port, invoice_cents: self.invoice_cents, ledger_tax_bps: self.ledger_tax_bps }
    }
    pub fn maybe_copy(&self) -> Option<Self> {
        Some(Self { smtp_host: self.smtp_host.clone(), smtp_port: self.smtp_port, invoice_cents: self.invoice_cents, ledger_tax_bps: self.ledger_tax_bps })
    }
    pub fn qualified_snapshot(&self) -> crate::BillingMailer {
        crate::BillingMailer { smtp_host: self.smtp_host.clone(), smtp_port: self.smtp_port, invoice_cents: self.invoice_cents, ledger_tax_bps: self.ledger_tax_bps }
    }
}
impl std::fmt::Display for BillingMailer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{} {}/{}", self.smtp_host, self.smtp_port, self.invoice_cents, self.ledger_tax_bps)
    }
}
#[cfg(test)]
mod tests {
    use super::BillingMailer;
    impl BillingMailer {
        fn test_bridge(&self) -> u64 { self.smtp_port as u64 + self.invoice_cents + self.ledger_tax_bps + self.smtp_host.len() as u64 }
    }
}
"#,
    );

    // Includes both `constructor` and `static` method accessing `this.*` (self_receiver = None).
    let ts_with_ctor_and_static = r#"
export class BillingMailer {
    private smtpHost: string; private smtpPort: number; private invoiceCents: number; private ledgerTaxBps: number;
    constructor(host: string, port: number, cents: number, bps: number) {
        this.smtpHost = host; this.smtpPort = port; this.invoiceCents = cents; this.ledgerTaxBps = bps;
    }
    static staticBridge(): number { return this.smtpPort + this.invoiceCents + this.ledgerTaxBps; }
    formatSmtpEndpoint(): string { return this.smtpHost + ":" + this.smtpPort; }
    smtpUsesTlsPort(): boolean { return this.smtpPort === 465 || this.smtpPort === 587; }
    configureSmtpRelay(host: string, port: number): void { if (host.length > 0) { this.smtpHost = host; this.smtpPort = port; } }
    computeInvoiceTax(): number { return Math.floor((this.invoiceCents * this.ledgerTaxBps) / 10000); }
    invoiceGrossTotal(): number { return this.invoiceCents + Math.floor((this.invoiceCents * this.ledgerTaxBps) / 10000); }
    postLedgerAdjustment(delta: number): void { if (delta > 0) { this.invoiceCents += delta; } }
}
"#;

    let swift_with_init_deinit = r#"
public protocol Auditable { func auditSummary() -> String }
public final class BillingMailer {
    private var smtpHost: String; private var smtpPort: Int; private var invoiceCents: Int; private var ledgerTaxBps: Int
    public init(host: String, port: Int, cents: Int, bps: Int) {
        self.smtpHost = host; self.smtpPort = port; self.invoiceCents = cents; self.ledgerTaxBps = bps
    }
    deinit { self.smtpHost = ""; self.smtpPort = 0; self.invoiceCents = 0; self.ledgerTaxBps = 0 }
    public func formatSmtpEndpoint() -> String { return "\(self.smtpHost):\(self.smtpPort)" }
    public func smtpUsesTlsPort() -> Bool { return self.smtpPort == 465 || self.smtpPort == 587 }
    public func configureSmtpRelay(host: String, port: Int) { if !host.isEmpty { self.smtpHost = host; self.smtpPort = port } }
    public func computeInvoiceTax() -> Int { return (self.invoiceCents * self.ledgerTaxBps) / 10000 }
    public func invoiceGrossTotal() -> Int { return self.invoiceCents + (self.invoiceCents * self.ledgerTaxBps) / 10000 }
    public func postLedgerAdjustment(delta: Int) { if delta > 0 { self.invoiceCents += delta } }
}
extension BillingMailer: Auditable {
    public func auditSummary() -> String { return "\(self.smtpHost):\(self.smtpPort):\(self.invoiceCents):\(self.ledgerTaxBps)" }
}
"#;

    for (label, path, src) in [
        ("rust glue+test", "src/lib.rs", rust_with_glue.as_str()),
        ("ts ctor+static", "src/billing.ts", ts_with_ctor_and_static),
        (
            "swift init+deinit+ext",
            "Sources/App/Billing.swift",
            swift_with_init_deinit,
        ),
    ] {
        let diags = analyze_project(&[(path, src)], &cfg)?;
        assert_eq!(
            diags_for(&diags, "low-cohesion-type").len(),
            1,
            "{label}: glue/static/test methods must NOT mask disjoint clusters: {diags:?}"
        );
    }
    Ok(())
}

#[test]
fn low_cohesion_type_retains_nontrivial_methods_with_accessor_names() -> Result<(), Box<dyn Error>>
{
    let src = r#"
pub struct DisguisedGod {
    smtp_host: String, smtp_port: u16, invoice_cents: u64, ledger_tax_bps: u64,
}
impl DisguisedGod {
    pub fn get_smtp_host(&self) -> String { self.smtp_host.clone() }
    pub fn get_smtp_port(&self) -> u16 { if self.smtp_host.is_empty() { 587 } else { self.smtp_port } }
    pub fn set_smtp_port(&mut self, delta: u16) { self.smtp_port += delta; }
    pub fn get_invoice_label(&self) -> String { format!("{}", self.invoice_cents) }
    pub fn get_ledger_tax(&self) -> u64 { self.invoice_cents + self.ledger_tax_bps }
}
"#;
    let diags = analyze_project(&[("src/disguised.rs", src)], &cohesion_only_config())?;
    assert_eq!(diags_for(&diags, "low-cohesion-type").len(), 1);
    Ok(())
}
