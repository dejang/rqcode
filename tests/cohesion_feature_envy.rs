mod support;

use rqcode::config::Config;
use rqcode::diagnostic::Severity;
use std::error::Error;
use support::{analyze_project, assert_no_rule, cohesion_only_config, diags_for};

fn fe_config(
    cohesion_section: &str,
    tc_section: &str,
    fe_section: &str,
) -> Result<Config, Box<dyn Error>> {
    let toml = format!(
        "[similarity]\nenabled = false\n[test_coupling]\nenabled = false\n[ast_grep]\nenabled = false\nbuiltin_rules = false\n[cohesion]\n{cohesion_section}\n[cohesion.type_cohesion]\n{tc_section}\n[cohesion.feature_envy]\n{fe_section}\n"
    );
    Ok(Config::from_toml(&toml)?)
}

fn invoice_fixture(lang: &str) -> &'static str {
    match lang {
        "ts" => r"export class InvoiceLedger {
    private invoiceSubtotal: number = 0;
    private invoiceTaxRate: number = 0;
    private invoiceDiscount: number = 0;
    invoiceSubtotalCents(): number { return this.invoiceSubtotal + this.invoiceDiscount; }
    invoiceTaxCents(): number { return Math.floor((this.invoiceSubtotal * this.invoiceTaxRate) / 100); }
    invoiceDiscountCents(): number { return this.invoiceDiscount; }
    applyInvoiceCredit(credit: number): void { this.invoiceDiscount += credit; }
}
",
        "swift" => r"public final class InvoiceLedger {
    private var invoiceSubtotal: Int = 0
    private var invoiceTaxRate: Int = 0
    private var invoiceDiscount: Int = 0
    public init() {}
    public func invoiceSubtotalCents() -> Int { return self.invoiceSubtotal + self.invoiceDiscount }
    public func invoiceTaxCents() -> Int { return (self.invoiceSubtotal * self.invoiceTaxRate) / 100 }
    public func invoiceDiscountCents() -> Int { return self.invoiceDiscount }
    public func applyInvoiceCredit(_ credit: Int) { self.invoiceDiscount += credit }
}
",
        _ => r"pub struct InvoiceLedger {
    pub invoice_subtotal: u64,
    pub invoice_tax_rate: u64,
    pub invoice_discount: u64,
}

impl InvoiceLedger {
    pub fn invoice_subtotal_cents(&self) -> u64 { self.invoice_subtotal + self.invoice_discount }
    pub fn invoice_tax_cents(&self) -> u64 { (self.invoice_subtotal * self.invoice_tax_rate) / 100 }
    pub fn invoice_discount_cents(&self) -> u64 { self.invoice_discount }
    pub fn apply_invoice_credit(&mut self, credit: u64) { self.invoice_discount += credit; }
}
",
    }
}

#[test]
fn feature_envy_flags_rust_parameter_and_local_variable_targets() -> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    let rust_report = analyze_project(
        &[
            ("src/invoice.rs", invoice_fixture("rs")),
            (
                "src/mailer.rs",
                r#"use crate::invoice::InvoiceLedger;

pub struct SmtpMailer {
    smtp_host: String,
    smtp_port: u16,
}

impl SmtpMailer {
    pub fn format_smtp_banner(&self) -> String {
        format!("{}:{}", self.smtp_host, self.smtp_port)
    }

    pub fn compute_invoice_payable(&self, ledger: &InvoiceLedger) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents()
    }
}
"#,
            ),
        ],
        &cfg,
    )?;

    let rust_diags = diags_for(&rust_report, "feature-envy");
    assert_eq!(
        rust_diags.len(),
        1,
        "Rust envious method should fire once: {rust_diags:#?}"
    );
    assert_eq!(rust_diags[0].severity(), Severity::Warning);
    assert_eq!(
        rust_diags[0].span().start_line(),
        13,
        "anchor must be method declaration line"
    );
    assert!(
        rust_diags[0].message().contains("compute_invoice_payable")
            && rust_diags[0].message().contains("InvoiceLedger"),
        "diagnostic message must name method and target type: {}",
        rust_diags[0].message()
    );
    assert!(
        rust_diags[0]
            .suggestion()
            .is_some_and(|s| s.contains("InvoiceLedger")),
        "suggestion must recommend moving onto target type: {:?}",
        rust_diags[0].suggestion()
    );
    assert!(
        rust_diags[0].related().iter().any(|r| {
            r.span().file().ends_with("invoice.rs")
                && r.span().start_line() == 1
                && r.label().contains("InvoiceLedger")
        }),
        "related location must point at target type definition: {:#?}",
        rust_diags[0].related()
    );

    let local_report = analyze_project(
        &[
            ("src/invoice.rs", invoice_fixture("rs")),
            (
                "src/mailer.rs",
                r"use crate::invoice::InvoiceLedger;
pub struct SmtpMailer { smtp_host: String }
impl SmtpMailer {
    pub fn compute_local_invoice_payable(&self) -> u64 {
        let ledger = InvoiceLedger { invoice_subtotal: 100, invoice_tax_rate: 10, invoice_discount: 5 };
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents()
    }
}
",
            ),
        ],
        &cfg,
    )?;
    assert_eq!(
        diags_for(&local_report, "feature-envy").len(),
        1,
        "local target must fire"
    );
    Ok(())
}

#[test]
fn feature_envy_flags_misplaced_methods_in_typescript_and_swift() -> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();
    let ts_report = analyze_project(
        &[
            ("src/invoice.ts", invoice_fixture("ts")),
            (
                "src/mailer.ts",
                r#"import { InvoiceLedger } from "./invoice";

export class SmtpMailer {
    private smtpHost: string = "";

    formatSmtpBanner(): string { return this.smtpHost; }

    computeInvoicePayable(ledger: InvoiceLedger): number {
        return ledger.invoiceSubtotalCents() + ledger.invoiceTaxCents() - ledger.invoiceDiscountCents();
    }
}
"#,
            ),
        ],
        &cfg,
    )?;
    let ts_diags = diags_for(&ts_report, "feature-envy");
    assert_eq!(
        ts_diags.len(),
        1,
        "TypeScript envious method should fire: {ts_diags:#?}"
    );
    assert_eq!(ts_diags[0].severity(), Severity::Warning);
    assert_eq!(ts_diags[0].span().start_line(), 8);
    assert!(
        ts_diags[0]
            .related()
            .iter()
            .any(|r| r.span().file().ends_with("invoice.ts") && r.span().start_line() == 1)
    );

    let swift_report = analyze_project(
        &[
            ("Sources/App/InvoiceLedger.swift", invoice_fixture("swift")),
            (
                "Sources/App/SmtpMailer.swift",
                r#"public final class SmtpMailer {
    private var smtpHost: String = ""

    public func formatSmtpBanner() -> String { return self.smtpHost }

    public func computeInvoicePayable(_ ledger: InvoiceLedger) -> Int {
        return ledger.invoiceSubtotalCents() + ledger.invoiceTaxCents() - ledger.invoiceDiscountCents()
    }
}
"#,
            ),
        ],
        &cfg,
    )?;
    let swift_diags = diags_for(&swift_report, "feature-envy");
    assert_eq!(
        swift_diags.len(),
        1,
        "Swift envious method should fire: {swift_diags:#?}"
    );
    assert_eq!(swift_diags[0].severity(), Severity::Warning);
    assert_eq!(swift_diags[0].span().start_line(), 6);
    assert!(
        swift_diags[0]
            .related()
            .iter()
            .any(|r| r.span().file().ends_with("InvoiceLedger.swift") && r.span().start_line() == 1)
    );
    Ok(())
}

#[test]
fn feature_envy_respects_advisory_severity_independent_toggles_and_suppression()
-> Result<(), Box<dyn Error>> {
    let files = [
        ("src/invoice.rs", invoice_fixture("rs")),
        (
            "src/mailer.rs",
            r#"use crate::invoice::InvoiceLedger;
pub struct SmtpMailer { smtp_host: String }
impl SmtpMailer {
    pub fn compute_invoice_payable(&self, ledger: &InvoiceLedger) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents()
    }
}
pub struct SplitOwner {
    smtp_host: String, smtp_port: u16, audit_code: u64, audit_tag: u64,
}
impl SplitOwner {
    pub fn smtp_addr(&self) -> String { format!("{}:{}", self.smtp_host, self.smtp_port) }
    pub fn smtp_tls(&self) -> bool { self.smtp_port == 465 }
    pub fn smtp_set(&mut self, h: String, p: u16) { if !h.is_empty() { self.smtp_host = h; self.smtp_port = p; } }
    pub fn audit_sum(&self) -> u64 { self.audit_code + self.audit_tag }
    pub fn audit_diff(&self) -> u64 { self.audit_code.saturating_sub(self.audit_tag) }
    pub fn audit_bump(&mut self, d: u64) { if d > 0 { self.audit_code += d; } }
}
"#,
        ),
    ];

    let err_cfg = fe_config("advisory_severity = \"error\"", "", "")?;
    let err_report = analyze_project(&files, &err_cfg)?;
    let err_diags = diags_for(&err_report, "feature-envy");
    assert_eq!(err_diags.len(), 1);
    assert_eq!(err_diags[0].severity(), Severity::Error);

    // Independent toggles: disabling type_cohesion must NOT disable feature_envy (corpus must still build)
    let tc_off = fe_config("", "enabled = false", "enabled = true")?;
    let r_tc_off = analyze_project(&files, &tc_off)?;
    assert_no_rule(&r_tc_off, "low-cohesion-type");
    assert_eq!(diags_for(&r_tc_off, "feature-envy").len(), 1);

    // Disabling feature_envy must NOT disable type_cohesion
    let fe_off = fe_config("", "enabled = true", "enabled = false")?;
    let r_fe_off = analyze_project(&files, &fe_off)?;
    assert_no_rule(&r_fe_off, "feature-envy");
    assert_eq!(diags_for(&r_fe_off, "low-cohesion-type").len(), 1);

    let cohesion_off = fe_config("enabled = false", "", "")?;
    assert_no_rule(&analyze_project(&files, &cohesion_off)?, "feature-envy");

    let suppressed = [
        ("src/invoice.rs", invoice_fixture("rs")),
        (
            "src/mailer.rs",
            r"use crate::invoice::InvoiceLedger;
pub struct SmtpMailer { smtp_host: String }
impl SmtpMailer {
    // rqcode:allow(feature-envy)
    pub fn compute_invoice_payable(&self, ledger: &InvoiceLedger) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents()
    }
}
",
        ),
    ];
    assert_no_rule(
        &analyze_project(&suppressed, &cohesion_only_config())?,
        "feature-envy",
    );
    Ok(())
}

#[test]
fn feature_envy_isolates_rust_method_eligibility_exemptions_per_owner() -> Result<(), Box<dyn Error>>
{
    let cfg = cohesion_only_config();
    // Each exempt method sits on its OWN owner struct so no sibling method pollutes v_own.
    let report = analyze_project(
        &[
            ("src/invoice.rs", invoice_fixture("rs")),
            (
                "src/mailer.rs",
                r"use crate::invoice::InvoiceLedger;
pub trait InvoiceCalc { fn compute_invoice_via_trait(&self, ledger: &InvoiceLedger) -> u64; }
pub fn free_compute_invoice(ledger: &InvoiceLedger) -> u64 {
    ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents()
}
pub struct EmptyMarker;
impl EmptyMarker {
    pub fn compute_invoice_stateless(&self, ledger: &InvoiceLedger) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents()
    }
}
pub struct TraitHost { smtp_host: String }
impl InvoiceCalc for TraitHost {
    fn compute_invoice_via_trait(&self, ledger: &InvoiceLedger) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents()
    }
}
pub struct StaticHost { smtp_host: String }
impl StaticHost {
    pub fn static_compute_invoice(ledger: &InvoiceLedger) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents()
    }
}
pub struct SelfFactory { smtp_host: String }
impl SelfFactory {
    pub fn with_invoice(&self, ledger: &InvoiceLedger) -> Self {
        let total = ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents();
        Self { smtp_host: total.to_string() }
    }
}
pub struct OptFactory { smtp_host: String }
impl OptFactory {
    pub fn try_with_invoice(&self, ledger: &InvoiceLedger) -> Option<Self> {
        let total = ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents();
        Some(Self { smtp_host: total.to_string() })
    }
}
pub struct QualFactory { smtp_host: String }
impl QualFactory {
    pub fn build_with_invoice(&self, ledger: &InvoiceLedger) -> crate::mailer::QualFactory {
        let total = ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents();
        QualFactory { smtp_host: total.to_string() }
    }
}
pub struct CfgTestHost { smtp_host: String }
impl CfgTestHost {
    #[cfg(test)]
    pub fn test_compute_invoice(&self, ledger: &InvoiceLedger) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents()
    }
}
pub struct FiringHost { smtp_host: String }
impl FiringHost {
    pub fn inherent_compute_invoice(&self, ledger: &InvoiceLedger) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents()
    }
}
",
            ),
            (
                "tests/helper.rs",
                r"use crate::invoice::InvoiceLedger;
pub struct TestFileHost { smtp_host: String }
impl TestFileHost {
    pub fn compute_invoice_in_test_file(&self, ledger: &InvoiceLedger) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents()
    }
}
",
            ),
        ],
        &cfg,
    )?;

    let diags = diags_for(&report, "feature-envy");
    assert_eq!(
        diags.len(),
        1,
        "only FiringHost::inherent_compute_invoice may fire: {diags:#?}"
    );
    assert!(diags[0].message().contains("inherent_compute_invoice"));
    Ok(())
}

#[test]
fn feature_envy_isolates_ts_and_swift_lifecycle_static_and_protocol_exemptions()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();
    let ts_report = analyze_project(
        &[
            ("src/invoice.ts", invoice_fixture("ts")),
            (
                "src/mailer.ts",
                r#"import { InvoiceLedger } from "./invoice";
export class CtorHost {
    private smtpHost: string;
    constructor(ledger: InvoiceLedger) {
        this.smtpHost = String(ledger.invoiceSubtotalCents() + ledger.invoiceTaxCents() - ledger.invoiceDiscountCents());
    }
}
export class StaticHost {
    private smtpHost: string = "";
    static staticComputeInvoice(ledger: InvoiceLedger): number {
        return ledger.invoiceSubtotalCents() + ledger.invoiceTaxCents() - ledger.invoiceDiscountCents();
    }
}
export class FiringHost {
    private smtpHost: string = "";
    instanceComputeInvoice(ledger: InvoiceLedger): number {
        return ledger.invoiceSubtotalCents() + ledger.invoiceTaxCents() - ledger.invoiceDiscountCents();
    }
}
"#,
            ),
        ],
        &cfg,
    )?;
    let ts_diags = diags_for(&ts_report, "feature-envy");
    assert_eq!(
        ts_diags.len(),
        1,
        "only TS FiringHost::instanceComputeInvoice may fire: {ts_diags:#?}"
    );
    assert!(ts_diags[0].message().contains("instanceComputeInvoice"));

    // Swift: separate classes for init, deinit (has_self_receiver == true, isolates is_lifecycle!),
    // protocol extension, and firing control.
    let swift_report = analyze_project(
        &[
            ("Sources/App/InvoiceLedger.swift", invoice_fixture("swift")),
            (
                "Sources/App/SmtpMailer.swift",
                r#"public protocol InvoiceCalc { func computeViaProtocol(_ ledger: InvoiceLedger) -> Int }
public final class InitHost {
    private var smtpHost: String = ""
    public init(_ ledger: InvoiceLedger) {
        self.smtpHost = "\(ledger.invoiceSubtotalCents() + ledger.invoiceTaxCents() - ledger.invoiceDiscountCents())"
    }
}
public final class DeinitHost {
    private var smtpHost: String = ""
    deinit {
        let ledger = InvoiceLedger()
        let _ = ledger.invoiceSubtotalCents() + ledger.invoiceTaxCents() - ledger.invoiceDiscountCents()
    }
}
public final class ProtoHost { private var smtpHost: String = "" }
extension ProtoHost: InvoiceCalc {
    public func computeViaProtocol(_ ledger: InvoiceLedger) -> Int {
        return ledger.invoiceSubtotalCents() + ledger.invoiceTaxCents() - ledger.invoiceDiscountCents()
    }
}
public final class FiringHost {
    private var smtpHost: String = ""
    public func computeInvoiceFiring(_ ledger: InvoiceLedger) -> Int {
        return ledger.invoiceSubtotalCents() + ledger.invoiceTaxCents() - ledger.invoiceDiscountCents()
    }
}
"#,
            ),
        ],
        &cfg,
    )?;
    let swift_diags = diags_for(&swift_report, "feature-envy");
    assert_eq!(
        swift_diags.len(),
        1,
        "only Swift FiringHost::computeInvoiceFiring may fire: {swift_diags:#?}"
    );
    assert!(swift_diags[0].message().contains("computeInvoiceFiring"));
    Ok(())
}

#[test]
fn feature_envy_deduplicates_with_external_field_mutation_per_func_target_pair()
-> Result<(), Box<dyn Error>> {
    let mutator_and_reader = r"use crate::invoice::InvoiceLedger;
pub struct SmtpMutator { smtp_host: String }
impl SmtpMutator {
    pub fn mutate_invoice_ledger(&self, ledger: &mut InvoiceLedger) {
        ledger.invoice_subtotal += 10;
        ledger.invoice_tax_rate += 1;
        ledger.invoice_discount += 2;
    }
}
pub struct SmtpReader { smtp_host: String }
impl SmtpReader {
    pub fn read_invoice_ledger(&self, ledger: &InvoiceLedger) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents()
    }
}
";

    // 1. Enabled: mutate_invoice_ledger deduped, while read_invoice_ledger targeting SAME InvoiceLedger STILL fires!
    let default_cfg = cohesion_only_config();
    let dedup_report = analyze_project(
        &[
            ("src/invoice.rs", invoice_fixture("rs")),
            ("src/mailer.rs", mutator_and_reader),
        ],
        &default_cfg,
    )?;
    assert_eq!(diags_for(&dedup_report, "external-field-mutation").len(), 1);
    let fe_dedup = diags_for(&dedup_report, "feature-envy");
    assert_eq!(
        fe_dedup.len(),
        1,
        "dedup must key by (func, target), not target alone: {fe_dedup:#?}"
    );
    assert!(fe_dedup[0].message().contains("read_invoice_ledger"));

    // 2. Inline suppression of external-field-mutation STILL keeps (mutate, InvoiceLedger) deduplicated
    let suppressed_mut = r"use crate::invoice::InvoiceLedger;
pub struct SmtpMutator { smtp_host: String }
impl SmtpMutator {
    // rqcode:allow(external-field-mutation)
    pub fn mutate_invoice_ledger(&self, ledger: &mut InvoiceLedger) {
        ledger.invoice_subtotal += 10;
        ledger.invoice_tax_rate += 1;
        ledger.invoice_discount += 2;
    }
}
";
    let suppressed_report = analyze_project(
        &[
            ("src/invoice.rs", invoice_fixture("rs")),
            ("src/mailer.rs", suppressed_mut),
        ],
        &default_cfg,
    )?;
    assert_no_rule(&suppressed_report, "external-field-mutation");
    assert_no_rule(&suppressed_report, "feature-envy");

    // 3. flag_external_field_mutation = false: BOTH mutate_invoice_ledger and read_invoice_ledger fire feature-envy
    let no_ext_mut_cfg = fe_config("flag_external_field_mutation = false", "", "")?;
    let enabled_report = analyze_project(
        &[
            ("src/invoice.rs", invoice_fixture("rs")),
            ("src/mailer.rs", mutator_and_reader),
        ],
        &no_ext_mut_cfg,
    )?;
    assert_no_rule(&enabled_report, "external-field-mutation");
    assert_eq!(diags_for(&enabled_report, "feature-envy").len(), 2);

    // 4. Cross-target precision: mutating AuditCounter does NOT suppress feature-envy on InvoiceLedger
    let cross_target_report = analyze_project(
        &[
            ("src/invoice.rs", invoice_fixture("rs")),
            (
                "src/audit.rs",
                r"pub struct AuditCounter { pub total_events: u64 }
impl AuditCounter { pub fn reset_audit(&mut self) { self.total_events = 0; } }
",
            ),
            (
                "src/mailer.rs",
                r"use crate::audit::AuditCounter;
use crate::invoice::InvoiceLedger;
pub struct SmtpMailer { smtp_host: String }
impl SmtpMailer {
    pub fn compute_invoice_and_audit(&self, ledger: &InvoiceLedger, audit: &mut AuditCounter) -> u64 {
        audit.total_events += 1;
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents()
    }
}
",
            ),
        ],
        &default_cfg,
    )?;
    assert_eq!(
        diags_for(&cross_target_report, "external-field-mutation").len(),
        1
    );
    assert_eq!(diags_for(&cross_target_report, "feature-envy").len(), 1);
    Ok(())
}
