mod support;

use rqcode::config::Config;
use std::error::Error;
use support::{analyze_project, assert_no_rule, cohesion_only_config, diags_for};

fn tc_toml(extra: &str) -> Result<Config, Box<dyn Error>> {
    let toml = format!(
        "[similarity]\nenabled = false\n[test_coupling]\nenabled = false\n[ast_grep]\nenabled = false\nbuiltin_rules = false\n[cohesion.type_cohesion]\n{extra}\n"
    );
    Ok(Config::from_toml(&toml)?)
}

#[test]
fn low_cohesion_type_bridges_two_way_and_transitive_three_way_clusters()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();
    let no_bridge = tc_toml("max_conceptual_similarity = 1.01")?;

    let two_way = r"
pub struct InvoiceBilling {
    billing_invoice_subtotal: u64, billing_invoice_discount: u64,
    billing_invoice_audit: u64, billing_invoice_ledger: u64,
}
impl InvoiceBilling {
    pub fn invoice_billing_subtotal(&self) -> u64 { self.billing_invoice_subtotal + 1 }
    pub fn invoice_billing_discount(&self) -> u64 { self.billing_invoice_subtotal + self.billing_invoice_discount }
    pub fn invoice_billing_net(&self) -> u64 { self.billing_invoice_subtotal + self.billing_invoice_discount + 2 }
    pub fn invoice_billing_audit(&self) -> u64 { self.billing_invoice_audit + 1 }
    pub fn invoice_billing_ledger(&self) -> u64 { self.billing_invoice_audit + self.billing_invoice_ledger }
    pub fn invoice_billing_checksum(&self) -> u64 { self.billing_invoice_audit + self.billing_invoice_ledger + 2 }
}
";
    assert_no_rule(
        &analyze_project(&[("src/inv.rs", two_way)], &cfg)?,
        "low-cohesion-type",
    );
    assert_eq!(
        diags_for(
            &analyze_project(&[("src/inv.rs", two_way)], &no_bridge)?,
            "low-cohesion-type"
        )
        .len(),
        1
    );

    let three_chain = r"
pub struct Pipeline {
    quartz_a: u64, quartz_b: u64, bridge_x: u64, bridge_y: u64, prism_a: u64, prism_b: u64,
}
impl Pipeline {
    pub fn alpha_quartz_prepare(&self) -> u64 { self.quartz_a + self.quartz_b }
    pub fn alpha_quartz_finalize(&mut self, v: u64) { self.quartz_a += v; }
    pub fn alpha_quartz_omega_prism_link(&self) -> u64 { self.bridge_x + self.bridge_y }
    pub fn alpha_quartz_omega_prism_sync(&mut self, v: u64) { self.bridge_x += v; }
    pub fn omega_prism_render(&self) -> u64 { self.prism_a + self.prism_b }
    pub fn omega_prism_flush(&mut self, v: u64) { self.prism_a += v; }
}
";
    assert_no_rule(
        &analyze_project(&[("src/pipe.rs", three_chain)], &cfg)?,
        "low-cohesion-type",
    );
    assert_eq!(
        diags_for(
            &analyze_project(&[("src/pipe.rs", three_chain)], &no_bridge)?,
            "low-cohesion-type"
        )
        .len(),
        1
    );
    Ok(())
}

#[test]
fn low_cohesion_type_handles_partial_bridging_and_declared_field_union_tokens()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();
    let no_bridge = tc_toml("max_conceptual_similarity = 1.01")?;

    // Partial bridging: A+B merge via `invoice`/`billing`, leaving C (`smtp_*`) disjoint -> 2 clusters (3 unbridged).
    let partial_three = r#"
pub struct PartialBridge {
    invoice_billing_subtotal: u64, invoice_billing_discount: u64,
    invoice_billing_audit: u64, invoice_billing_ledger: u64,
    smtp_host: String, smtp_port: u16,
}
impl PartialBridge {
    pub fn invoice_billing_net(&self) -> u64 { self.invoice_billing_subtotal + self.invoice_billing_discount }
    pub fn invoice_billing_cap(&mut self, v: u64) { self.invoice_billing_subtotal += v; }
    pub fn invoice_billing_verify(&self) -> u64 { self.invoice_billing_audit + self.invoice_billing_ledger }
    pub fn invoice_billing_post(&mut self, v: u64) { self.invoice_billing_audit += v; }
    pub fn format_smtp_endpoint(&self) -> String { format!("{}:{}", self.smtp_host, self.smtp_port) }
    pub fn smtp_uses_tls_port(&self) -> bool { self.smtp_port == 465 || self.smtp_port == 587 }
}
"#;
    let bridged = analyze_project(&[("src/partial.rs", partial_three)], &cfg)?;
    let b_hits = diags_for(&bridged, "low-cohesion-type");
    assert_eq!(b_hits.len(), 1);
    assert!(b_hits[0].message().contains("2 disjoint"));
    assert_eq!(b_hits[0].related().len(), 2);
    let merged_label = b_hits[0].related()[0].label();
    assert!(
        merged_label.contains("invoice_billing_subtotal")
            && merged_label.contains("invoice_billing_audit")
    );
    assert!(b_hits[0].related()[1].label().contains("smtp_host"));
    let unbridged = analyze_project(&[("src/partial.rs", partial_three)], &no_bridge)?;
    assert_eq!(
        diags_for(&unbridged, "low-cohesion-type")[0]
            .related()
            .len(),
        3
    );

    // Pin declared field names from union F*(m) inside T(C_a): eligible methods share ZERO tokens,
    // while trivial getters `ra`/`rb` (excluded from M_elig) propagate `quartz_ledger_{a,b}` into F*(m).
    let field_only_bridge = r"
pub struct FieldUnionBridge { quartz_ledger_a: u8, quartz_ledger_b: u8, p1: u8, p2: u8 }
impl FieldUnionBridge {
    pub fn ra(&self) -> u8 { self.quartz_ledger_a }
    pub fn rb(&self) -> u8 { self.quartz_ledger_b }
    pub fn m_a1(&self) -> u8 { self.ra() + self.p1 }
    pub fn m_a2(&self) -> u8 { self.ra() + 1 }
    pub fn m_a3(&mut self, v: u8) { self.p1 += v; }
    pub fn m_b1(&self) -> u8 { self.rb() + self.p2 }
    pub fn m_b2(&mut self, v: u8) { self.p2 += v; }
}
";
    assert_no_rule(
        &analyze_project(&[("src/fb.rs", field_only_bridge)], &cfg)?,
        "low-cohesion-type",
    );
    assert_eq!(
        diags_for(
            &analyze_project(&[("src/fb.rs", field_only_bridge)], &no_bridge)?,
            "low-cohesion-type"
        )
        .len(),
        1
    );
    Ok(())
}

#[test]
fn low_cohesion_type_respects_tcc_denominator_and_strict_threshold_boundaries()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    let core_plus_aux = r#"
pub struct DominantCore { order_total: u64, order_tax: u64, smtp_host: String, smtp_port: u16 }
impl DominantCore {
    pub fn m1(&self) -> u64 { self.order_total + 1 }
    pub fn m2(&self) -> u64 { self.order_total + 2 }
    pub fn m3(&self) -> u64 { self.order_total + 3 }
    pub fn m4(&self) -> u64 { self.order_total + 4 }
    pub fn m5(&self) -> u64 { self.order_total + 5 }
    pub fn m6(&self) -> u64 { self.order_total + 6 }
    pub fn m7(&self) -> u64 { self.order_total + self.order_tax }
    pub fn m8(&self) -> u64 { self.order_total * 2 }
    pub fn smtp_a(&self) -> String { format!("{}:{}", self.smtp_host, self.smtp_port) }
    pub fn smtp_b(&self) -> bool { self.smtp_port == 465 }
    pub fn util_a(&self, x: u64) -> u64 { x + 1 }
    pub fn util_b(&self, x: u64) -> u64 { x + 2 }
    pub fn util_c(&self, x: u64) -> u64 { x + 3 }
}
"#;
    assert_no_rule(
        &analyze_project(&[("src/core.rs", core_plus_aux)], &cfg)?,
        "low-cohesion-type",
    );
    assert_eq!(
        diags_for(
            &analyze_project(
                &[("src/core.rs", core_plus_aux)],
                &tc_toml("max_tcc = 0.70")?
            )?,
            "low-cohesion-type"
        )
        .len(),
        1
    );

    // Exact 3+3 split (TCC = 0.40, cos = 0.0): fires at default, silent at max_tcc = 0.40 and max_conceptual_similarity = 0.0.
    let exact_boundary = r#"
pub struct ExactSplit { smtp_host: String, smtp_port: u16, invoice_cents: u64, ledger_tax_bps: u64 }
impl ExactSplit {
    pub fn smtp_a(&self) -> String { format!("{}:{}", self.smtp_host, self.smtp_port) }
    pub fn smtp_b(&self) -> bool { self.smtp_port == 465 }
    pub fn smtp_c(&mut self, p: u16) { self.smtp_port += p; }
    pub fn inv_a(&self) -> u64 { self.invoice_cents + self.ledger_tax_bps }
    pub fn inv_b(&self) -> u64 { self.invoice_cents * self.ledger_tax_bps }
    pub fn inv_c(&mut self, d: u64) { self.invoice_cents += d; }
}
"#;
    assert_eq!(
        diags_for(
            &analyze_project(&[("src/b.rs", exact_boundary)], &cfg)?,
            "low-cohesion-type"
        )
        .len(),
        1
    );
    assert_no_rule(
        &analyze_project(&[("src/b.rs", exact_boundary)], &tc_toml("max_tcc = 0.40")?)?,
        "low-cohesion-type",
    );
    assert_no_rule(
        &analyze_project(
            &[("src/b.rs", exact_boundary)],
            &tc_toml("max_conceptual_similarity = 0.0")?,
        )?,
        "low-cohesion-type",
    );
    Ok(())
}

#[test]
fn low_cohesion_type_excludes_orphan_and_stateless_components() -> Result<(), Box<dyn Error>> {
    let orphan = r"
pub struct WithOrphan { order_total: u64, order_tax: u64, order_fee: u64, smtp_port: u16 }
impl WithOrphan {
    pub fn a(&self) -> u64 { self.order_total + 1 }
    pub fn b(&self) -> u64 { self.order_total + self.order_tax }
    pub fn c(&self) -> u64 { self.order_total + self.order_fee }
    pub fn d(&self) -> u64 { self.order_total * 2 }
    pub fn orphan_smtp(&self) -> bool { self.smtp_port == 587 }
}
";
    assert_no_rule(
        &analyze_project(&[("src/o.rs", orphan)], &tc_toml("max_tcc = 0.70")?)?,
        "low-cohesion-type",
    );
    assert_eq!(
        diags_for(
            &analyze_project(
                &[("src/o.rs", orphan)],
                &tc_toml("min_component_methods = 1\nmax_tcc = 0.70")?
            )?,
            "low-cohesion-type"
        )
        .len(),
        1
    );

    let stateless_comp = r"
pub struct OneStatefulCluster { a: u64, b: u64, c: u64, d: u64 }
impl OneStatefulCluster {
    pub fn f1(&self) -> u64 { self.a + self.b }
    pub fn f2(&self) -> u64 { self.a + self.c }
    pub fn f3(&self) -> u64 { self.a + self.d }
    pub fn f4(&mut self, v: u64) { self.a += v; }
    pub fn helper_one(&self, x: u64) -> u64 { self.helper_two(x) + 1 }
    pub fn helper_two(&self, x: u64) -> u64 { x * 2 }
}
";
    assert_no_rule(
        &analyze_project(&[("src/s.rs", stateless_comp)], &tc_toml("max_tcc = 1.01")?)?,
        "low-cohesion-type",
    );
    Ok(())
}

#[test]
fn low_cohesion_type_enforces_configurable_accessor_test_mod_and_test_file_gates()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    let mostly_accessors = r#"
pub struct ConfigHolder { smtp_host: String, smtp_port: u16, invoice_cents: u64, ledger_tax_bps: u64 }
impl ConfigHolder {
    pub fn smtp_host(&self) -> &str { &self.smtp_host }
    pub fn smtp_port(&self) -> u16 { self.smtp_port }
    pub fn smtp_port_mut(&mut self) -> &mut u16 { &mut self.smtp_port }
    pub fn set_invoice_cents(&mut self, v: u64) { self.invoice_cents = v; }
    pub fn compute_tax(&self) -> u64 { (self.invoice_cents * self.ledger_tax_bps) / 10_000 }
    pub fn format_relay(&self) -> String { format!("{}:{}", self.smtp_host, self.smtp_port) }
}
"#;
    assert_no_rule(
        &analyze_project(&[("src/h.rs", mostly_accessors)], &cfg)?,
        "low-cohesion-type",
    );
    assert_eq!(
        diags_for(
            &analyze_project(
                &[("src/h.rs", mostly_accessors)],
                &tc_toml("max_accessor_nodes = 0")?
            )?,
            "low-cohesion-type"
        )
        .len(),
        1
    );

    // Same god type: fires in `src/helper.rs`, silent inside `tests/helper.rs` (`file.is_test_file()`)
    // and inside `#[cfg(test)] mod tests`.
    let god_body = r"
pub struct TestGod { smtp_a: u64, smtp_b: u64, inv_c: u64, inv_d: u64 }
impl TestGod {
    pub fn smtp_m1(&self) -> u64 { self.smtp_a + self.smtp_b }
    pub fn smtp_m2(&self) -> u64 { self.smtp_a * self.smtp_b }
    pub fn smtp_m3(&mut self, v: u64) { self.smtp_a += v; }
    pub fn inv_m4(&self) -> u64 { self.inv_c + self.inv_d }
    pub fn inv_m5(&self) -> u64 { self.inv_c * self.inv_d }
    pub fn inv_m6(&mut self, v: u64) { self.inv_c += v; }
}
";
    assert_eq!(
        diags_for(
            &analyze_project(&[("src/helper.rs", god_body)], &cfg)?,
            "low-cohesion-type"
        )
        .len(),
        1
    );
    assert_no_rule(
        &analyze_project(&[("tests/helper.rs", god_body)], &cfg)?,
        "low-cohesion-type",
    );
    let in_test_mod = format!("#[cfg(test)]\nmod tests {{\n{god_body}\n}}\n");
    assert_no_rule(
        &analyze_project(&[("src/t.rs", &in_test_mod)], &cfg)?,
        "low-cohesion-type",
    );
    Ok(())
}

#[test]
fn low_cohesion_type_enforces_configurable_field_and_method_size_gates()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    let three_fields = r#"
pub struct ThreeFields { smtp_host: String, invoice_cents: u64, ledger_tax_bps: u64 }
impl ThreeFields {
    pub fn f1(&self) -> String { format!("{}", self.smtp_host) }
    pub fn f2(&self) -> bool { !self.smtp_host.is_empty() }
    pub fn f3(&mut self, h: String) { if !h.is_empty() { self.smtp_host = h; } }
    pub fn f4(&self) -> u64 { self.invoice_cents + self.ledger_tax_bps }
    pub fn f5(&self) -> u64 { self.invoice_cents * self.ledger_tax_bps }
    pub fn f6(&mut self, d: u64) { self.invoice_cents += d; }
}
"#;
    assert_no_rule(
        &analyze_project(&[("src/f3.rs", three_fields)], &cfg)?,
        "low-cohesion-type",
    );
    assert_eq!(
        diags_for(
            &analyze_project(&[("src/f3.rs", three_fields)], &tc_toml("min_fields = 3")?)?,
            "low-cohesion-type"
        )
        .len(),
        1
    );

    let four_methods = r#"
pub struct FourMethods { smtp_host: String, smtp_port: u16, invoice_cents: u64, ledger_tax_bps: u64 }
impl FourMethods {
    pub fn a(&self) -> String { format!("{}:{}", self.smtp_host, self.smtp_port) }
    pub fn b(&self) -> bool { self.smtp_port == 465 }
    pub fn c(&self) -> u64 { self.invoice_cents + self.ledger_tax_bps }
    pub fn d(&mut self, v: u64) { self.invoice_cents += v; }
}
"#;
    assert_no_rule(
        &analyze_project(&[("src/m4.rs", four_methods)], &cfg)?,
        "low-cohesion-type",
    );
    assert_eq!(
        diags_for(
            &analyze_project(&[("src/m4.rs", four_methods)], &tc_toml("min_methods = 4")?)?,
            "low-cohesion-type"
        )
        .len(),
        1
    );

    let four_field_using = format!(
        "{four_methods}\nimpl FourMethods {{ pub fn stateless(&self, x: u64) -> u64 {{ x + 1 }} }}\n"
    );
    assert_eq!(
        diags_for(
            &analyze_project(&[("src/g.rs", &four_field_using)], &cfg)?,
            "low-cohesion-type"
        )
        .len(),
        1
    );
    assert_no_rule(
        &analyze_project(
            &[("src/g.rs", &four_field_using)],
            &tc_toml("min_field_using_methods = 5")?,
        )?,
        "low-cohesion-type",
    );
    Ok(())
}
