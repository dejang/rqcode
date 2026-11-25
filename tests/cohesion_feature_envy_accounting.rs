mod support;

use rqcode::config::Config;
use std::error::Error;
use support::{analyze_project, assert_no_rule, cohesion_only_config, diags_for};

fn fe_toml(fe_section: &str) -> Result<Config, Box<dyn Error>> {
    let toml = format!(
        "[similarity]\nenabled = false\n[test_coupling]\nenabled = false\n[ast_grep]\nenabled = false\nbuiltin_rules = false\n[cohesion]\n[cohesion.feature_envy]\n{fe_section}\n"
    );
    Ok(Config::from_toml(&toml)?)
}

fn rust_invoice_mod(extra_methods: &str) -> String {
    format!(
        r"pub struct InvoiceLedger {{
    pub invoice_subtotal: u64,
    pub invoice_tax_rate: u64,
    pub invoice_discount: u64,
    pub invoice_penalty: u64,
}}

impl InvoiceLedger {{
    pub fn invoice_subtotal_cents(&self) -> u64 {{ self.invoice_subtotal }}
    pub fn invoice_tax_cents(&self) -> u64 {{ (self.invoice_subtotal * self.invoice_tax_rate) / 100 }}
    pub fn invoice_discount_cents(&self) -> u64 {{ self.invoice_discount }}
    pub fn invoice_penalty_cents(&self) -> u64 {{ self.invoice_penalty }}
    pub fn apply_invoice_credit(&mut self, credit: u64) {{ self.invoice_discount += credit; }}
    {extra_methods}
}}
"
    )
}

#[test]
fn feature_envy_counts_distinct_members_and_ignores_containers_and_scoped_calls()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();
    let inv = rust_invoice_mod(
        "pub fn default_invoice_rate() -> u64 { 10 }\n    pub fn min_invoice_total() -> u64 { 1 }\n    pub fn max_invoice_total() -> u64 { 999 }",
    );

    // Separate owner per case so no sibling pollutes v_own:
    // - RepeatHost: 3 calls to same member -> A_foreign = 1 < 3 (silent)
    // - ContainerHost: Vec/Option methods + static associated calls -> A_foreign = 0 (silent)
    // - ScopedHost: UFCS CallKind::Scoped calls to real instance methods in Methods(T) -> ignored by CallKind::Method filter (silent)
    // - DistinctHost: 3 distinct instance method calls -> fires!
    let report = analyze_project(
        &[
            ("src/invoice.rs", &inv),
            (
                "src/mailer.rs",
                r"use crate::invoice::InvoiceLedger;
pub struct RepeatHost { smtp_host: String }
impl RepeatHost {
    pub fn repeat_invoice_subtotal(&self, ledger: &InvoiceLedger) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_subtotal_cents() + ledger.invoice_subtotal_cents()
    }
}
pub struct ContainerHost { smtp_host: String }
impl ContainerHost {
    pub fn inspect_invoice_containers(&self, batch: &mut Vec<InvoiceLedger>, maybe: &Option<InvoiceLedger>) -> u64 {
        batch.clear();
        let a = batch.len() as u64;
        let b = u64::from(batch.is_empty());
        let c = u64::from(maybe.is_some());
        let d = InvoiceLedger::default_invoice_rate() + InvoiceLedger::min_invoice_total() + InvoiceLedger::max_invoice_total();
        a + b + c + d
    }
}
pub struct ScopedHost { smtp_host: String }
impl ScopedHost {
    pub fn scoped_invoice_calls(&self, ledger: &InvoiceLedger) -> u64 {
        InvoiceLedger::invoice_subtotal_cents(ledger)
            + InvoiceLedger::invoice_tax_cents(ledger)
            + InvoiceLedger::invoice_discount_cents(ledger)
    }
}
pub struct DistinctHost { smtp_host: String }
impl DistinctHost {
    pub fn distinct_invoice_members(&self, ledger: &InvoiceLedger) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() + ledger.invoice_discount_cents()
    }
}
",
            ),
        ],
        &cfg,
    )?;
    let diags = diags_for(&report, "feature-envy");
    assert_eq!(diags.len(), 1, "only DistinctHost may fire: {diags:#?}");
    assert!(diags[0].message().contains("distinct_invoice_members"));
    Ok(())
}

#[test]
fn feature_envy_ignores_ts_arrays_statics_unknown_fields_and_non_stateful_enums()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    // TS: Array methods + static calls + unknown foreign field accesses (not in Fields(T) U Methods(T)) -> silent;
    // foreign property-syntax getters (in Methods(T)) -> FIRES!
    let ts_report = analyze_project(
        &[
            (
                "src/invoice.ts",
                r"export class InvoiceLedger {
    private invoiceSubtotal: number = 0;
    private invoiceTaxRate: number = 0;
    private invoiceDiscount: number = 0;
    get invoiceSubtotalProp(): number { return this.invoiceSubtotal; }
    get invoiceTaxProp(): number { return this.invoiceTaxRate; }
    get invoiceDiscountProp(): number { return this.invoiceDiscount; }
    applyInvoiceCredit(credit: number): void { this.invoiceSubtotal -= credit; }
    static defaultInvoiceRate(): number { return 10; }
    static minInvoiceTotal(): number { return 1; }
    static maxInvoiceTotal(): number { return 999; }
}
",
            ),
            (
                "src/mailer.ts",
                r#"import { InvoiceLedger } from "./invoice";
export class ArrayStaticHost {
    private smtpHost: string = "";
    inspectInvoiceBatch(items: InvoiceLedger[]): number {
        const mapped = items.map((x) => x);
        const filtered = items.filter((x) => Boolean(x));
        const sliced = items.slice(0, 1);
        return mapped.length + filtered.length + sliced.length
            + InvoiceLedger.defaultInvoiceRate()
            + InvoiceLedger.minInvoiceTotal()
            + InvoiceLedger.maxInvoiceTotal();
    }
}
export class UnknownFieldHost {
    private smtpHost: string = "";
    computeInvoiceMissing(ledger: InvoiceLedger): number {
        return Number((ledger as any).invoiceSubtotalBogus)
            + Number((ledger as any).invoiceTaxBogus)
            + Number((ledger as any).invoiceDiscountBogus);
    }
}
export class ForeignGetterHost {
    private smtpHost: string = "";
    computeInvoiceProps(ledger: InvoiceLedger): number {
        return ledger.invoiceSubtotalProp + ledger.invoiceTaxProp - ledger.invoiceDiscountProp;
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
        "only ForeignGetterHost may fire: {ts_diags:#?}"
    );
    assert!(ts_diags[0].message().contains("computeInvoiceProps"));

    // Non-stateful foreign enum WITH a behavioral parameter-taking method (so Case B cannot mask it) -> silent
    let enum_report = analyze_project(
        &[
            (
                "src/status.rs",
                r"pub enum InvoiceStatus { Draft, Paid }
impl InvoiceStatus {
    pub fn invoice_code(&self) -> u64 { 1 }
    pub fn invoice_label(&self) -> u64 { 2 }
    pub fn invoice_priority(&self) -> u64 { 3 }
    pub fn apply_step(&mut self, delta: u64) -> u64 { delta }
}
",
            ),
            (
                "src/mailer.rs",
                r"use crate::status::InvoiceStatus;
pub struct SmtpMailer { smtp_host: String }
impl SmtpMailer {
    pub fn compute_invoice_status(&self, status: &InvoiceStatus) -> u64 {
        status.invoice_code() + status.invoice_label() + status.invoice_priority()
    }
}
",
            ),
        ],
        &cfg,
    )?;
    assert_no_rule(&enum_report, "feature-envy");
    Ok(())
}

#[test]
fn feature_envy_counts_ts_self_getters_with_firing_control()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    // 1. TS self getter counts in A_self:
    //    - WithGetterHost: 1 field + 1 self getter vs 3 foreign -> LAA = 2/5 = 0.40 >= 0.34 (silent)
    //    - WithoutGetterHost (control): 1 field vs 3 foreign -> LAA = 1/4 = 0.25 < 0.34 (fires)
    let ts_getter_report = analyze_project(
        &[
            (
                "src/invoice.ts",
                r"export class InvoiceLedger {
    private invoiceSubtotal: number = 0;
    private invoiceTaxRate: number = 0;
    private invoiceDiscount: number = 0;
    invoiceSubtotalCents(): number { return this.invoiceSubtotal; }
    invoiceTaxCents(): number { return this.invoiceTaxRate; }
    invoiceDiscountCents(): number { return this.invoiceDiscount; }
    applyInvoiceCredit(credit: number): void { this.invoiceDiscount += credit; }
}
",
            ),
            (
                "src/mailer.ts",
                r#"import { InvoiceLedger } from "./invoice";
export class WithGetterHost {
    private smtpPort: number = 587;
    get smtpLabel(): number { return this.smtpPort; }
    computeInvoiceWithGetter(ledger: InvoiceLedger): number {
        return this.smtpPort + this.smtpLabel + ledger.invoiceSubtotalCents() + ledger.invoiceTaxCents() - ledger.invoiceDiscountCents();
    }
}
export class WithoutGetterHost {
    private smtpPort: number = 587;
    computeInvoiceNoGetter(ledger: InvoiceLedger): number {
        return this.smtpPort + ledger.invoiceSubtotalCents() + ledger.invoiceTaxCents() - ledger.invoiceDiscountCents();
    }
}
"#,
            ),
        ],
        &cfg,
    )?;
    let ts_getter_diags = diags_for(&ts_getter_report, "feature-envy");
    assert_eq!(
        ts_getter_diags.len(),
        1,
        "only WithoutGetterHost may fire: {ts_getter_diags:#?}"
    );
    assert!(
        ts_getter_diags[0]
            .message()
            .contains("computeInvoiceNoGetter")
    );

    Ok(())
}

#[test]
fn feature_envy_counts_self_calls_distinct_self_and_excludes_non_self_owner()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();
    // Rust A_self self-call arm + A_self distinctness + non-self same-type peer exclusion (T != T_own):
    //    - SelfCallHost: 1 self field + 1 self call vs 3 foreign -> LAA = 2/5 = 0.40 (silent, isolates CallReceiver::SelfInstance)
    //    - DupSelfHost: same self field read twice -> A_self = 1 (not 2!), LAA = 1/4 = 0.25 (fires, isolates A_self distinctness)
    //    - PeerHost: 3 peer fields (short names so v_m stays closer to InvoiceLedger) + 3 on InvoiceLedger + 1 on TaxHelper:
    //      if peer counted as A_self -> LAA = 3/7 = 0.43 (silent); if peer counted as A_foreign -> FDP = 3 > 2 (silent);
    //      correct -> A_self = 0, FDP = 2, LAA = 0.0 -> FIRES!
    let inv = rust_invoice_mod("");
    let rust_self_report = analyze_project(
        &[
            ("src/invoice.rs", &inv),
            (
                "src/tax.rs",
                r"pub struct TaxHelper { pub tax_basis: u64 }
impl TaxHelper {
    pub fn tax_amount(&self) -> u64 { self.tax_basis }
    pub fn set_tax(&mut self, v: u64) { self.tax_basis = v; }
}
",
            ),
            (
                "src/mailer.rs",
                r"use crate::invoice::InvoiceLedger;
use crate::tax::TaxHelper;
pub struct Quiet { q: u64 }
impl Quiet {
    pub fn qx(&self) -> u64 { self.q + 1 }
    pub fn invoice_with_self_call(&self, ledger: &InvoiceLedger) -> u64 {
        self.q + self.qx()
            + ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() + ledger.invoice_discount_cents()
    }
}
pub struct Muted { z: u64 }
impl Muted {
    pub fn invoice_with_dup_self(&self, ledger: &InvoiceLedger) -> u64 {
        self.z + self.z
            + ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() + ledger.invoice_discount_cents()
    }
}
pub struct Relay { smtp_host: String, x1: u64, x2: u64, x3: u64 }
impl Relay {
    pub fn format_smtp_banner(&self) -> String { self.smtp_host.clone() }
    pub fn invoice_with_peer(&self, p: &Relay, ledger: &InvoiceLedger, tax: &TaxHelper) -> u64 {
        p.x1 + p.x2 + p.x3 + tax.tax_amount()
            + ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() + ledger.invoice_discount_cents()
    }
}
",
            ),
        ],
        &cfg,
    )?;
    let self_diags = diags_for(&rust_self_report, "feature-envy");
    assert_eq!(
        self_diags.len(),
        2,
        "DupSelfHost and PeerHost must fire while SelfCallHost is silent: {self_diags:#?}"
    );
    assert!(
        self_diags
            .iter()
            .any(|d| d.message().contains("invoice_with_dup_self"))
    );
    assert!(
        self_diags
            .iter()
            .any(|d| d.message().contains("invoice_with_peer"))
    );
    Ok(())
}

#[test]
fn feature_envy_breaks_foreign_target_ties_deterministically_by_type_id()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();
    let make_pair = |first: &str, second: &str| {
        format!(
            r"pub struct {first} {{ pub invoice_alpha: u64, pub invoice_beta: u64, pub invoice_gamma: u64 }}
impl {first} {{
    pub fn invoice_one(&self) -> u64 {{ self.invoice_alpha }}
    pub fn invoice_two(&self) -> u64 {{ self.invoice_beta }}
    pub fn invoice_three(&self) -> u64 {{ self.invoice_gamma }}
    pub fn mutate_invoice(&mut self, v: u64) {{ self.invoice_alpha += v; }}
}}
pub struct {second} {{ pub invoice_alpha: u64, pub invoice_beta: u64, pub invoice_gamma: u64 }}
impl {second} {{
    pub fn invoice_one(&self) -> u64 {{ self.invoice_alpha }}
    pub fn invoice_two(&self) -> u64 {{ self.invoice_beta }}
    pub fn invoice_three(&self) -> u64 {{ self.invoice_gamma }}
    pub fn mutate_invoice(&mut self, v: u64) {{ self.invoice_alpha += v; }}
}}
"
        )
    };
    let caller = r"use crate::ledgers::{AlphaInvoice, BetaInvoice};
pub struct SmtpMailer { smtp_host: String }
impl SmtpMailer {
    pub fn reconcile_invoices(&self, a: &AlphaInvoice, b: &BetaInvoice) -> u64 {
        a.invoice_one() + a.invoice_two() + a.invoice_three() + b.invoice_one() + b.invoice_two() + b.invoice_three()
    }
}
";

    let alpha_first = analyze_project(
        &[
            ("src/ledgers.rs", &make_pair("AlphaInvoice", "BetaInvoice")),
            ("src/mailer.rs", caller),
        ],
        &cfg,
    )?;
    let d_alpha = diags_for(&alpha_first, "feature-envy");
    assert_eq!(d_alpha.len(), 1);
    assert!(d_alpha[0].message().contains("AlphaInvoice"));

    let beta_first = analyze_project(
        &[
            ("src/ledgers.rs", &make_pair("BetaInvoice", "AlphaInvoice")),
            ("src/mailer.rs", caller),
        ],
        &cfg,
    )?;
    let d_beta = diags_for(&beta_first, "feature-envy");
    assert_eq!(d_beta.len(), 1);
    assert!(d_beta[0].message().contains("BetaInvoice"));
    Ok(())
}

#[test]
fn feature_envy_enforces_min_foreign_accesses_on_dominant_target_not_total()
-> Result<(), Box<dyn Error>> {
    let default_cfg = cohesion_only_config();
    let inv = rust_invoice_mod("");
    let files = [
        ("src/invoice.rs", inv.as_str()),
        (
            "src/tax.rs",
            r"pub struct TaxHelper { pub tax_basis: u64 }
impl TaxHelper {
    pub fn tax_amount(&self) -> u64 { self.tax_basis }
    pub fn set_tax(&mut self, v: u64) { self.tax_basis = v; }
}
",
        ),
        (
            "src/mailer.rs",
            r"use crate::invoice::InvoiceLedger;
use crate::tax::TaxHelper;
pub struct TwoOnlyHost { smtp_port: u64 }
impl TwoOnlyHost {
    pub fn two_invoice_accesses(&self, ledger: &InvoiceLedger) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents()
    }
}
pub struct TwoPlusOneHost { smtp_port: u64 }
impl TwoPlusOneHost {
    pub fn two_plus_one_accesses(&self, ledger: &InvoiceLedger, tax: &TaxHelper) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() + tax.tax_amount()
    }
}
pub struct ThreeHost { smtp_port: u64 }
impl ThreeHost {
    pub fn three_invoice_accesses(&self, ledger: &InvoiceLedger) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents()
    }
}
",
        ),
    ];

    // Default min_foreign_accesses = 3:
    // - TwoOnlyHost (A_foreign(T*) = 2) -> silent
    // - TwoPlusOneHost (A_foreign(T*) = 2, A_foreign_total = 3) -> silent! (must gate on A_foreign(T*), not total)
    // - ThreeHost (A_foreign(T*) = 3) -> fires!
    let r_default = analyze_project(&files, &default_cfg)?;
    let d_default = diags_for(&r_default, "feature-envy");
    assert_eq!(
        d_default.len(),
        1,
        "only ThreeHost may fire: {d_default:#?}"
    );
    assert!(d_default[0].message().contains("three_invoice_accesses"));

    let min_four_cfg = fe_toml("min_foreign_accesses = 4")?;
    assert_no_rule(&analyze_project(&files, &min_four_cfg)?, "feature-envy");
    Ok(())
}

#[test]
fn feature_envy_enforces_fdp_gate_total_laa_denominator_and_strict_laa_bound()
-> Result<(), Box<dyn Error>> {
    let default_cfg = cohesion_only_config();
    let inv = rust_invoice_mod("");
    let helpers = r"pub struct TaxHelper { pub tax_basis: u64 }
impl TaxHelper {
    pub fn tax_amount(&self) -> u64 { self.tax_basis }
    pub fn set_tax(&mut self, v: u64) { self.tax_basis = v; }
}
pub struct FeeHelper { pub fee_basis: u64 }
impl FeeHelper {
    pub fn fee_amount(&self) -> u64 { self.fee_basis }
    pub fn set_fee(&mut self, v: u64) { self.fee_basis = v; }
}
";

    // Separate owner per case so v_own is never polluted:
    // - TwoProvHost: 2 self (field + method), 3 on InvoiceLedger, 1 on TaxHelper -> A_foreign_total = 4, LAA = 2/6 = 0.333 < 0.34, FDP = 2 -> FIRES!
    // - ThreeProvHost: 0 self, 3 on InvoiceLedger, 1 on TaxHelper, 1 on FeeHelper -> FDP = 3 > 2 -> SILENT by default, FIRES with max_foreign_providers = 3!
    let fdp_files = [
        ("src/invoice.rs", inv.as_str()),
        ("src/helpers.rs", helpers),
        (
            "src/mailer.rs",
            r"use crate::helpers::{FeeHelper, TaxHelper};
use crate::invoice::InvoiceLedger;
pub struct TwoProvHost { smtp_port: u64 }
impl TwoProvHost {
    pub fn smtp_offset(&self) -> u64 { self.smtp_port + 1 }
    pub fn invoice_two_providers(&self, ledger: &InvoiceLedger, tax: &TaxHelper) -> u64 {
        self.smtp_port + self.smtp_offset() + ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() + ledger.invoice_discount_cents() + tax.tax_amount()
    }
}
pub struct ThreeProvHost { smtp_port: u64 }
impl ThreeProvHost {
    pub fn invoice_three_providers(&self, ledger: &InvoiceLedger, tax: &TaxHelper, fee: &FeeHelper) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() + ledger.invoice_discount_cents() + tax.tax_amount() + fee.fee_amount()
    }
}
",
        ),
    ];
    let r_fdp_default = analyze_project(&fdp_files, &default_cfg)?;
    let d_fdp_default = diags_for(&r_fdp_default, "feature-envy");
    assert_eq!(
        d_fdp_default.len(),
        1,
        "FDP=2 with LAA=2/6 must fire while FDP=3 is silent: {d_fdp_default:#?}"
    );
    assert!(d_fdp_default[0].message().contains("invoice_two_providers"));

    let fdp_three_cfg = fe_toml("max_foreign_providers = 3")?;
    assert_eq!(
        diags_for(
            &analyze_project(&fdp_files, &fdp_three_cfg)?,
            "feature-envy"
        )
        .len(),
        2
    );

    // Strict `< max_laa`: separate owners for 1-self (LAA = 1/4 = 0.25) and 2-self (LAA = 2/5 = 0.40)
    let laa_files = [
        ("src/invoice.rs", inv.as_str()),
        (
            "src/mailer.rs",
            r"use crate::invoice::InvoiceLedger;
pub struct OneSelfHost { smtp_port: u64 }
impl OneSelfHost {
    pub fn invoice_one_self(&self, ledger: &InvoiceLedger) -> u64 {
        self.smtp_port + ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() + ledger.invoice_discount_cents()
    }
}
pub struct TwoSelfHost { smtp_port: u64, smtp_retries: u64 }
impl TwoSelfHost {
    pub fn invoice_two_self(&self, ledger: &InvoiceLedger) -> u64 {
        self.smtp_port + self.smtp_retries + ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() + ledger.invoice_discount_cents()
    }
}
",
        ),
    ];
    assert_no_rule(
        &analyze_project(&laa_files, &fe_toml("max_laa = 0.25")?)?,
        "feature-envy",
    );
    let r_laa = analyze_project(&laa_files, &fe_toml("max_laa = 0.26")?)?;
    let d_laa = diags_for(&r_laa, "feature-envy");
    assert_eq!(d_laa.len(), 1);
    assert!(d_laa[0].message().contains("invoice_one_self"));
    Ok(())
}
