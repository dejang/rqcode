mod support;

use rqcode::config::Config;
use std::error::Error;
use support::{analyze_project, assert_no_rule, cohesion_only_config, diags_for};

fn caller_for_invoice(owner_name: &str) -> String {
    format!(
        r"use crate::invoice::InvoiceLedger;
pub struct {owner_name} {{ smtp_host: String }}
impl {owner_name} {{
    pub fn compute_invoice_payable(&self, ledger: &InvoiceLedger) -> u64 {{
        ledger.invoice_subtotal + ledger.invoice_tax_rate + ledger.invoice_discount
    }}
}}
"
    )
}

#[test]
fn feature_envy_exempts_same_module_private_helpers_only() -> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    let make_same_module = |vis: &str| {
        format!(
            r"{vis}struct InvoiceScratch {{
    pub invoice_subtotal: u64,
    pub invoice_tax_rate: u64,
    pub invoice_discount: u64,
}}
impl InvoiceScratch {{
    fn invoice_subtotal_cents(&self) -> u64 {{ self.invoice_subtotal }}
    fn invoice_tax_cents(&self) -> u64 {{ self.invoice_tax_rate }}
    fn invoice_discount_cents(&self) -> u64 {{ self.invoice_discount }}
    fn apply_invoice_credit(&mut self, credit: u64) {{ self.invoice_discount += credit; }}
}}
pub struct SmtpMailer {{ smtp_host: String }}
impl SmtpMailer {{
    pub fn compute_invoice_payable(&self, scratch: &InvoiceScratch) -> u64 {{
        scratch.invoice_subtotal_cents() + scratch.invoice_tax_cents() - scratch.invoice_discount_cents()
    }}
}}
"
        )
    };

    // 1. Visibility::Private in same module -> EXEMPT
    assert_no_rule(
        &analyze_project(&[("src/mailer.rs", &make_same_module(""))], &cfg)?,
        "feature-envy",
    );

    // 2. Visibility::Public in same module -> FIRES
    let pub_same = analyze_project(&[("src/mailer.rs", &make_same_module("pub "))], &cfg)?;
    assert_eq!(
        diags_for(&pub_same, "feature-envy").len(),
        1,
        "public same-module target must fire"
    );

    // 3. Visibility::Internal (`pub(crate)`) in same module -> FIRES
    let internal_same =
        analyze_project(&[("src/mailer.rs", &make_same_module("pub(crate) "))], &cfg)?;
    assert_eq!(
        diags_for(&internal_same, "feature-envy").len(),
        1,
        "pub(crate) same-module target must fire"
    );

    // 4. Visibility::Private in DIFFERENT module (parent `billing` -> child `billing::mailer` via `super::`) -> FIRES!
    let cross_mod_private = analyze_project(
        &[
            (
                "src/billing/mod.rs",
                r"pub mod mailer;
struct HiddenLedger {
    pub invoice_subtotal: u64,
    pub invoice_tax_rate: u64,
    pub invoice_discount: u64,
}
impl HiddenLedger {
    fn invoice_subtotal_cents(&self) -> u64 { self.invoice_subtotal }
    fn invoice_tax_cents(&self) -> u64 { self.invoice_tax_rate }
    fn invoice_discount_cents(&self) -> u64 { self.invoice_discount }
    fn apply_invoice_credit(&mut self, credit: u64) { self.invoice_discount += credit; }
}
",
            ),
            (
                "src/billing/mailer.rs",
                r"use super::HiddenLedger;
pub struct SmtpMailer { smtp_host: String }
impl SmtpMailer {
    pub fn compute_invoice_payable(&self, ledger: &HiddenLedger) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents()
    }
}
",
            ),
        ],
        &cfg,
    )?;
    assert_eq!(
        diags_for(&cross_mod_private, "feature-envy").len(),
        1,
        "private target in different ModulePath must NOT be exempt"
    );
    Ok(())
}

#[test]
fn feature_envy_exempts_zero_method_dtos_with_all_exposed_fields_and_ignores_non_inherent_methods()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();
    let caller = caller_for_invoice("SmtpMailer");

    // Case A exempt: 0 non-test inherent instance methods (has static fn, trait impl, #[cfg(test)] mutator) + all fields pub
    let passive_dto = analyze_project(
        &[
            (
                "src/invoice.rs",
                r"pub trait Resettable { fn reset_invoice(&mut self); }
pub struct InvoiceLedger {
    pub invoice_subtotal: u64,
    pub invoice_tax_rate: u64,
    pub invoice_discount: u64,
}
impl Resettable for InvoiceLedger {
    fn reset_invoice(&mut self) { self.invoice_subtotal = 0; }
}
impl InvoiceLedger {
    pub fn create_default() -> Self { Self { invoice_subtotal: 0, invoice_tax_rate: 0, invoice_discount: 0 } }
    #[cfg(test)]
    pub fn test_mutate(&mut self) { self.invoice_subtotal = 0; }
}
",
            ),
            ("src/mailer.rs", &caller),
        ],
        &cfg,
    )?;
    assert_no_rule(&passive_dto, "feature-envy");

    // Case A NOT exempt (cross-module): 0 instance methods, 1 private field -> exposed_field_count() < fields.len()
    let partial_private = analyze_project(
        &[
            (
                "src/invoice.rs",
                r"pub struct InvoiceLedger {
    pub invoice_subtotal: u64,
    pub invoice_tax_rate: u64,
    pub invoice_discount: u64,
    invoice_secret_salt: u64,
}
",
            ),
            ("src/mailer.rs", &caller),
        ],
        &cfg,
    )?;
    assert_eq!(diags_for(&partial_private, "feature-envy").len(), 1);

    // Case A NOT exempt (same-module public type): 0 methods, private fields
    let same_mod_private_fields = analyze_project(
        &[(
            "src/mailer.rs",
            r"pub struct InvoiceLedger { invoice_subtotal: u64, invoice_tax_rate: u64, invoice_discount: u64 }
pub struct SmtpMailer { smtp_host: String }
impl SmtpMailer {
    pub fn compute_invoice_payable(&self, ledger: &InvoiceLedger) -> u64 {
        ledger.invoice_subtotal + ledger.invoice_tax_rate + ledger.invoice_discount
    }
}
",
        )],
        &cfg,
    )?;
    assert_eq!(diags_for(&same_mod_private_fields, "feature-envy").len(), 1);

    // No fallback when dominant T* is exempt: 4 accesses on exempt passive DTO + 3 accesses on behavioral secondary -> SILENT
    let no_fallback = analyze_project(
        &[
            (
                "src/dto.rs",
                r"pub struct InvoiceDto { pub a: u64, pub b: u64, pub c: u64, pub d: u64 }
",
            ),
            (
                "src/invoice.rs",
                r"pub struct InvoiceLedger { pub invoice_subtotal: u64, pub invoice_tax_rate: u64, pub invoice_discount: u64 }
impl InvoiceLedger {
    pub fn invoice_subtotal_cents(&self) -> u64 { self.invoice_subtotal }
    pub fn invoice_tax_cents(&self) -> u64 { self.invoice_tax_rate }
    pub fn invoice_discount_cents(&self) -> u64 { self.invoice_discount }
    pub fn apply_invoice_credit(&mut self, credit: u64) { self.invoice_discount += credit; }
}
",
            ),
            (
                "src/mailer.rs",
                r"use crate::dto::InvoiceDto;
use crate::invoice::InvoiceLedger;
pub struct SmtpMailer { smtp_host: String }
impl SmtpMailer {
    pub fn compute_invoice_from_dto(&self, dto: &InvoiceDto, ledger: &InvoiceLedger) -> u64 {
        dto.a + dto.b + dto.c + dto.d
            + ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() + ledger.invoice_discount_cents()
    }
}
",
            ),
        ],
        &cfg,
    )?;
    assert_no_rule(&no_fallback, "feature-envy");
    Ok(())
}

fn rust_non_behavioral_target(extra_method: &str) -> String {
    format!(
        r"pub struct InvoiceLedger {{
    pub invoice_subtotal: u64,
    pub invoice_tax_rate: u64,
    pub invoice_discount: u64,
    pub items: Vec<u64>,
}}
impl InvoiceLedger {{
    pub fn with_subtotal(&self, subtotal: u64) -> Self {{
        Self {{ invoice_subtotal: subtotal, invoice_tax_rate: self.invoice_tax_rate, invoice_discount: self.invoice_discount, items: Vec::new() }}
    }}
    pub fn rebuild_qualified(&self) -> crate::invoice::InvoiceLedger {{
        InvoiceLedger {{ invoice_subtotal: self.invoice_subtotal, invoice_tax_rate: self.invoice_tax_rate, invoice_discount: self.invoice_discount, items: Vec::new() }}
    }}
    pub fn build_with_loop(&self) -> Self {{
        loop {{
            break Self {{ invoice_subtotal: self.invoice_subtotal, invoice_tax_rate: self.invoice_tax_rate, invoice_discount: self.invoice_discount, items: Vec::new() }};
        }}
    }}
    pub fn set_subtotal(&mut self, v: u64) {{ self.invoice_subtotal = v; }}
    pub fn item_count(&self) -> usize {{ self.items.len() }}
    pub fn invoice_is_zero(&self) -> bool {{ self.invoice_subtotal == 0 }}
    pub fn invoice_ping(&self) -> bool {{ self.invoice_pong() }}
    pub fn invoice_pong(&self) -> bool {{ self.invoice_ping() || self.invoice_is_zero() }}
    {extra_method}
}}
"
    )
}

#[test]
fn feature_envy_exempts_non_behavioral_rust_structs_and_retains_every_behavioral_flip()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();
    let caller = caller_for_invoice("SmtpMailer");

    // Baseline: factories (Self, qualified, loop-factory), trivial setter, &self query calling self.items.len(), cyclic queries -> EXEMPT!
    assert_no_rule(
        &analyze_project(
            &[
                ("src/invoice.rs", &rust_non_behavioral_target("")),
                ("src/mailer.rs", &caller),
            ],
            &cfg,
        )?,
        "feature-envy",
    );

    // Disabling trivial-accessor budget (`max_accessor_nodes = 0`) strips set_subtotal's exemption -> FIRES!
    let zero_acc_cfg = Config::from_toml(
        "[similarity]\nenabled = false\n[test_coupling]\nenabled = false\n[ast_grep]\nenabled = false\nbuiltin_rules = false\n[cohesion.type_cohesion]\nmax_accessor_nodes = 0\n",
    )?;
    assert_eq!(
        diags_for(
            &analyze_project(
                &[
                    ("src/invoice.rs", &rust_non_behavioral_target("")),
                    ("src/mailer.rs", &caller)
                ],
                &zero_acc_cfg
            )?,
            "feature-envy"
        )
        .len(),
        1,
        "trivial-accessor arm in Case B must respect max_accessor_nodes"
    );

    // Each isolated behavioral flip must cause feature-envy to FIRE
    let flips = [
        (
            "mut_self_non_accessor",
            "pub fn touch_ledger(&mut self) -> u64 { self.invoice_subtotal + 1 }",
        ),
        (
            "non_empty_param",
            "pub fn scaled_subtotal(&self, factor: u64) -> u64 { self.invoice_subtotal * factor }",
        ),
        (
            "loop_without_assign",
            "pub fn loop_only(&self) -> u64 { loop { break self.invoice_subtotal; } }",
        ),
        (
            "assign_in_query",
            "pub fn assign_local(&self) -> u64 { let mut acc = self.invoice_subtotal; acc = acc + 1; acc }",
        ),
        (
            "ambient_io_in_query",
            "pub fn has_env_override(&self) -> bool { std::env::var(\"LEDGER_MODE\").is_ok() }",
        ),
        (
            "transitive_loop_via_exempt_factory",
            "pub fn query_via_factory(&self) -> u64 { self.build_with_loop().invoice_subtotal }",
        ),
    ];
    for (label, extra_method) in flips {
        let report = analyze_project(
            &[
                ("src/invoice.rs", &rust_non_behavioral_target(extra_method)),
                ("src/mailer.rs", &caller),
            ],
            &cfg,
        )?;
        assert_eq!(
            diags_for(&report, "feature-envy").len(),
            1,
            "Rust behavioral flip `{label}` must fire feature-envy"
        );
    }
    Ok(())
}

#[test]
fn feature_envy_exempts_non_behavioral_ts_classes_and_retains_every_behavioral_flip()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();
    let ts_caller = r#"import { InvoiceLedger } from "./invoice";
export class SmtpMailer {
    private smtpHost: string = "";
    computeInvoicePayable(ledger: InvoiceLedger): number {
        return ledger.invoiceSubtotal + ledger.invoiceTaxRate + ledger.invoiceDiscount;
    }
}
"#;
    let build_ts_target = |extra_method: &str| {
        format!(
            r"export class InvoiceLedger {{
    public invoiceSubtotal: number = 0;
    public invoiceTaxRate: number = 0;
    public invoiceDiscount: number = 0;
    private items: number[] = [];
    private handlers: (() => void)[] = [];
    private onFlush: () => void = () => {{}};
    private sink: {{ write(): void; isZero(): boolean }} = {{ write() {{}}, isZero() {{ return true; }} }};

    constructor(subtotal: number) {{ this.invoiceSubtotal = subtotal; }}
    get subtotal(): number {{ return this.invoiceSubtotal; }}
    setSubtotal(v: number): void {{ this.invoiceSubtotal = v; }}
    isZero(): boolean {{ return this.invoiceSubtotal === 0; }}
    isEmpty(): boolean {{ return this.items.length === 0; }}
    isPositive(): boolean {{ return !this.isZero(); }}
    ping(): boolean {{ return this.pong(); }}
    pong(): boolean {{ return this.ping() || this.isZero(); }}
    {extra_method}
}}
"
        )
    };

    // Baseline: constructor + trivial getter + trivial setter + 0-arg queries + cyclic queries -> EXEMPT
    assert_no_rule(
        &analyze_project(
            &[
                ("src/invoice.ts", &build_ts_target("")),
                ("src/mailer.ts", ts_caller),
            ],
            &cfg,
        )?,
        "feature-envy",
    );

    // Flips: indirect call, external call, stored closure `this.onFlush()` (not in I(T*)), direct write,
    // and non-vacuous transitive write via individually-exempt trivial setter `this.setSubtotal(0)`!
    let ts_flips = [
        (
            "indirect_call",
            "runHandler(): void { this.handlers[0](); }",
        ),
        ("external_call", "flush(): void { this.sink.write(); }"),
        ("external_call_same_name_as_self_query", "callSinkIsZero(): boolean { return this.sink.isZero(); }"),
        (
            "stored_closure_self_call",
            "callClosure(): void { this.onFlush(); }",
        ),
        (
            "direct_write",
            "clear(): void { this.invoiceSubtotal = 0; }",
        ),
        (
            "transitive_write_via_trivial_setter",
            "reset(): void { this.setSubtotal(0); }",
        ),
    ];
    for (label, extra_method) in ts_flips {
        let report = analyze_project(
            &[
                ("src/invoice.ts", &build_ts_target(extra_method)),
                ("src/mailer.ts", ts_caller),
            ],
            &cfg,
        )?;
        assert_eq!(
            diags_for(&report, "feature-envy").len(),
            1,
            "TS behavioral flip `{label}` must fire feature-envy"
        );
    }
    Ok(())
}

#[test]
fn feature_envy_exempts_non_behavioral_swift_structs_and_classes_and_retains_flips()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();
    let swift_caller = r#"public final class SmtpMailer {
    private var smtpHost: String = ""
    public func computeInvoicePayable(_ ledger: InvoiceLedger) -> Int {
        return ledger.invoiceSubtotal + ledger.invoiceTaxRate + ledger.invoiceDiscount
    }
}
"#;

    // 1. Swift struct: pure 0-arg query exempt; adding non-writing `mutating func` fires (isolates SharedRef!)
    let swift_struct = |extra: &str| {
        format!(
            r"public struct InvoiceLedger {{
    public var invoiceSubtotal: Int = 0
    public var invoiceTaxRate: Int = 0
    public var invoiceDiscount: Int = 0
    public func isZero() -> Bool {{ return self.invoiceSubtotal == 0 }}
    {extra}
}}
"
        )
    };
    assert_no_rule(
        &analyze_project(
            &[
                ("Sources/App/InvoiceLedger.swift", &swift_struct("")),
                ("Sources/App/SmtpMailer.swift", swift_caller),
            ],
            &cfg,
        )?,
        "feature-envy",
    );
    let swift_mut = analyze_project(
        &[
            (
                "Sources/App/InvoiceLedger.swift",
                &swift_struct(
                    "public mutating func touchLedger() -> Int { return self.invoiceSubtotal + 1 }",
                ),
            ),
            ("Sources/App/SmtpMailer.swift", swift_caller),
        ],
        &cfg,
    )?;
    assert_eq!(diags_for(&swift_mut, "feature-envy").len(), 1);

    // 2. Swift class: pure query + side-effecting `deinit` (isolates Case B is_lifecycle!) -> EXEMPT
    let swift_class_base = r"public final class InvoiceSink { public func write() {} }
public final class InvoiceLedger {
    public var invoiceSubtotal: Int = 0
    public var invoiceTaxRate: Int = 0
    public var invoiceDiscount: Int = 0
    private let sink = InvoiceSink()
    public func isZero() -> Bool { return self.invoiceSubtotal == 0 }
    deinit { self.sink.write() }
}
";
    assert_no_rule(
        &analyze_project(
            &[
                ("Sources/App/InvoiceLedger.swift", swift_class_base),
                ("Sources/App/SmtpMailer.swift", swift_caller),
            ],
            &cfg,
        )?,
        "feature-envy",
    );

    // 3. Swift class extension with external call (extension gets SharedRef; must key on TypeKind::Class!) -> FIRES
    let swift_class_ext = format!(
        "{swift_class_base}\nextension InvoiceLedger {{\n    public func flush() {{ self.sink.write() }}\n}}\n"
    );
    let r_ext = analyze_project(
        &[
            ("Sources/App/InvoiceLedger.swift", &swift_class_ext),
            ("Sources/App/SmtpMailer.swift", swift_caller),
        ],
        &cfg,
    )?;
    assert_eq!(
        diags_for(&r_ext, "feature-envy").len(),
        1,
        "Swift class extension calling external sink must trigger behavioral classification"
    );
    Ok(())
}
