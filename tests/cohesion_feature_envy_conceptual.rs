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

fn rust_invoice_mod(extra_field: &str) -> String {
    format!(
        r"pub struct InvoiceLedger {{
    pub invoice_subtotal: u64,
    pub invoice_tax_rate: u64,
    pub invoice_discount: u64,
    {extra_field}
}}
impl InvoiceLedger {{
    pub fn invoice_subtotal_cents(&self) -> u64 {{ self.invoice_subtotal }}
    pub fn invoice_tax_cents(&self) -> u64 {{ (self.invoice_subtotal * self.invoice_tax_rate) / 100 }}
    pub fn invoice_discount_cents(&self) -> u64 {{ self.invoice_discount }}
    pub fn apply_invoice_credit(&mut self, credit: u64) {{ self.invoice_discount += credit; }}
}}
"
    )
}

#[test]
fn feature_envy_excludes_self_method_from_owner_and_fires_when_both_cosines_nonzero()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();
    let inv = rust_invoice_mod("");

    // 1. Self-exclusion of `m` from v_own (single-method owner)
    let self_excl = analyze_project(
        &[
            ("src/invoice.rs", &inv),
            (
                "src/mailer.rs",
                r"use crate::invoice::InvoiceLedger;
pub struct SmtpMailer { smtp_host: String }
impl SmtpMailer {
    pub fn compute_invoice_payable(&self, ledger: &InvoiceLedger) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents()
    }
}
",
            ),
        ],
        &cfg,
    )?;
    assert_eq!(
        diags_for(&self_excl, "feature-envy").len(),
        1,
        "m must be excluded from v_own so a single-method owner does not self-correlate"
    );

    // 2. Positive where BOTH cos(v_m, v_own) > 0 AND cos(v_m, v_{T*}) > cos(v_m, v_own) >= 0.15
    //    `render` is shared with SmtpMailer's other method (cos(own) > 0), while invoice tokens dominate T*
    let both_nonzero = analyze_project(
        &[
            ("src/invoice.rs", &inv),
            (
                "src/mailer.rs",
                r"use crate::invoice::InvoiceLedger;
pub struct SmtpMailer { smtp_host: String }
impl SmtpMailer {
    pub fn render_smtp_banner(&self) -> String { self.smtp_host.clone() }
    pub fn render_invoice_payable(&self, ledger: &InvoiceLedger) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents()
    }
}
",
            ),
        ],
        &cfg,
    )?;
    assert_eq!(
        diags_for(&both_nonzero, "feature-envy").len(),
        1,
        "positive with non-zero cos(v_m, v_own) < cos(v_m, v_target) must fire"
    );
    Ok(())
}

#[test]
fn feature_envy_enforces_strict_greater_than_owner_with_firing_control()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();
    let bucket_mod = r"pub struct VaultBucket { pub a: u64, pub b: u64, pub c: u64 }
impl VaultBucket {
    pub fn ra(&self) -> u64 { self.a }
    pub fn rb(&self) -> u64 { self.b }
    pub fn rc(&self) -> u64 { self.c }
    pub fn mutate(&mut self, v: u64) { self.a += v; }
}
";

    // 1. cos(v_m, v_VaultBucket) >= 0.15 (via `vb: &VaultBucket`), YET cos(v_m, v_own) > cos(v_m, v_VaultBucket) -> SILENT!
    let closer_to_own = analyze_project(
        &[
            ("src/storage.rs", bucket_mod),
            (
                "src/mailer.rs",
                r"use crate::storage::VaultBucket;
pub struct SmtpDispatchQueue {
    smtp_dispatch_relay: String,
    smtp_dispatch_retry: u64,
}
impl SmtpDispatchQueue {
    pub fn enqueue_smtp_dispatch_header(&self) -> u64 { self.smtp_dispatch_retry }
    pub fn schedule_smtp_dispatch_retry(&self) -> u64 { self.smtp_dispatch_retry + 1 }
    pub fn enqueue_smtp_dispatch_batch(&self, vb: &VaultBucket) -> u64 {
        let smtp_dispatch_delay = vb.ra() + vb.rb() + vb.rc();
        smtp_dispatch_delay
    }
}
",
            ),
        ],
        &cfg,
    )?;
    assert_no_rule(&closer_to_own, "feature-envy");

    // 2. Firing control: identical method on unrelated owner PlainHost -> cos(v_m, v_own) = 0 < cos(v_m, v_VaultBucket) >= 0.15 -> FIRES!
    let firing_control = analyze_project(
        &[
            ("src/storage.rs", bucket_mod),
            (
                "src/mailer.rs",
                r"use crate::storage::VaultBucket;
pub struct PlainHost { x: u64 }
impl PlainHost {
    pub fn enqueue_smtp_dispatch_batch(&self, vb: &VaultBucket) -> u64 {
        let smtp_dispatch_delay = vb.ra() + vb.rb() + vb.rc();
        smtp_dispatch_delay
    }
}
",
            ),
        ],
        &cfg,
    )?;
    assert_eq!(
        diags_for(&firing_control, "feature-envy").len(),
        1,
        "control proves cos(v_m, v_VaultBucket) >= 0.15 so strict `>` alone silenced closer_to_own"
    );

    // 3. Exact zero-cosine tie (cos(m, T*) == cos(m, own) == 0.0) with min_conceptual_affinity = 0.0 -> STILL SILENT (strict `>`)!
    let zero_tie = analyze_project(
        &[
            (
                "src/target.rs",
                r"pub struct X { pub a: u64, pub b: u64, pub c: u64 }
impl X {
    pub fn f1(&self) -> u64 { self.a }
    pub fn f2(&self) -> u64 { self.b }
    pub fn f3(&self) -> u64 { self.c }
    pub fn m(&mut self, v: u64) { self.a += v; }
}
",
            ),
            (
                "src/owner.rs",
                r"use crate::target::X;
pub struct Y { pub d: u64 }
impl Y {
    pub fn g(&self, x: &X) -> u64 { x.f1() + x.f2() + x.f3() }
}
",
            ),
        ],
        &fe_toml("min_conceptual_affinity = 0.0")?,
    )?;
    assert_no_rule(&zero_tie, "feature-envy");
    Ok(())
}

#[test]
fn feature_envy_enforces_default_affinity_floor_and_excludes_cfg_test_from_owner_vector()
-> Result<(), Box<dyn Error>> {
    let default_cfg = cohesion_only_config();

    // 1. Weak positive affinity: 0.0 == cos(v_m, v_own) < cos(v_m, v_Vault) < 0.15 (only `vault` shared amidst ~12 distinct tokens each).
    //    Must be SILENT at default min_conceptual_affinity = 0.15, and FIRE when lowered to 0.05!
    let weak_files = [
        (
            "src/vault.rs",
            r"pub struct Vault { pub alpha_one: u64, pub beta_two: u64, pub gamma_three: u64 }
impl Vault {
    pub fn ra(&self) -> u64 { self.alpha_one }
    pub fn rb(&self) -> u64 { self.beta_two }
    pub fn rc(&self) -> u64 { self.gamma_three }
    pub fn mutate_kappa(&mut self, v: u64) { self.alpha_one += v; }
}
",
        ),
        (
            "src/host.rs",
            r"use crate::vault::Vault;
pub struct PlainHost { x: u64 }
impl PlainHost {
    pub fn quux_plugh_xyzzy_corge(&self, vb: &Vault) -> u64 {
        let thud_wibble = vb.ra() + vb.rb() + vb.rc();
        thud_wibble
    }
}
",
        ),
    ];
    assert_no_rule(&analyze_project(&weak_files, &default_cfg)?, "feature-envy");
    assert_eq!(
        diags_for(
            &analyze_project(&weak_files, &fe_toml("min_conceptual_affinity = 0.05")?)?,
            "feature-envy"
        )
        .len(),
        1,
        "weak positive affinity (0 < cos < 0.15) must fire once floor is lowered below cos"
    );

    // 2. #[cfg(test)] method on SmtpMailer loaded with invoice tokens must NOT pollute v_own
    let inv = rust_invoice_mod("");
    let cfg_test_files = [
        ("src/invoice.rs", inv.as_str()),
        (
            "src/mailer.rs",
            r"use crate::invoice::InvoiceLedger;
pub struct SmtpMailer { smtp_host: String }
impl SmtpMailer {
    pub fn compute_invoice_payable(&self, ledger: &InvoiceLedger) -> u64 {
        ledger.invoice_subtotal_cents() + ledger.invoice_tax_cents() - ledger.invoice_discount_cents()
    }
    #[cfg(test)]
    pub fn test_compute_invoice_subtotal_tax_discount_ledger_payable(&self) -> u64 {
        let (invoice_subtotal_cents, invoice_tax_cents, invoice_discount_cents) = (1, 2, 3);
        invoice_subtotal_cents + invoice_tax_cents + invoice_discount_cents
    }
}
",
        ),
    ];
    assert_eq!(
        diags_for(
            &analyze_project(&cfg_test_files, &default_cfg)?,
            "feature-envy"
        )
        .len(),
        1,
        "#[cfg(test)] methods must not pollute v_own"
    );
    assert_no_rule(
        &analyze_project(&cfg_test_files, &fe_toml("min_conceptual_affinity = 1.01")?)?,
        "feature-envy",
    );
    Ok(())
}
