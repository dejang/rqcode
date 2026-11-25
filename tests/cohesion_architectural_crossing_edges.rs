mod support;

use rqcode::config::Config;
use rqcode::diagnostic::Diagnostic;
use std::error::Error;
use std::fmt::Write as _;
use std::path::Path;
use support::{analyze_project, assert_no_rule, cohesion_only_config, diags_for};

struct EdgeFixture {
    files: Vec<(String, String)>,
}

impl EdgeFixture {
    fn new() -> Self {
        Self { files: Vec::new() }
    }

    fn add(&mut self, path: impl Into<String>, src: impl Into<String>) {
        self.files.push((path.into(), src.into()));
    }

    fn add_rust(&mut self, name: &str, imports: &[&str], body: &str) {
        let mut src = String::new();
        for dep in imports {
            let _ = writeln!(src, "use crate::{dep};");
        }
        src.push_str(body);
        self.add(format!("src/{name}.rs"), src);
    }

    fn add_ts(&mut self, name: &str, imports: &[&str], body: &str) {
        let mut src = String::new();
        for (idx, dep) in imports.iter().enumerate() {
            let _ = writeln!(src, "import {{ sym as s{idx} }} from \"{dep}\";");
        }
        src.push_str(body);
        self.add(format!("src/{name}.ts"), src);
    }

    fn analyze(&self, cfg: &Config) -> Result<Vec<Diagnostic>, Box<dyn Error>> {
        let refs: Vec<(&str, &str)> = self
            .files
            .iter()
            .map(|(p, c)| (p.as_str(), c.as_str()))
            .collect();
        analyze_project(&refs, cfg)
    }
}

#[test]
fn architectural_crossing_exempt_high_fan_nodes_do_not_skew_quantile_distribution()
-> Result<(), Box<dyn Error>> {
    // 13 candidates (`knot` has (4,4), candidate sorted[10]=4) + 3 exempt nodes
    // (`bin/main.rs`, `pkg/lib.rs`, `facade/mod.rs`) with (6,6). Exempt nodes must not enter
    // U_cand (which would raise p=13 to 6 and suppress `knot`).
    let mut skew = EdgeFixture::new();
    for i in 0..6 {
        skew.add_rust(
            &format!("out_{i}"),
            &[],
            &format!("pub fn o_{i}() -> u64 {{ {i} }}\n"),
        );
    }
    skew.add_rust(
        "knot",
        &["out_0", "out_1", "out_2", "out_3"],
        "pub fn k() -> u64 { out_0::o_0() + out_1::o_1() + out_2::o_2() + out_3::o_3() }\n",
    );
    for i in 0..4 {
        skew.add_rust(
            &format!("in_{i}"),
            &["knot", "bin", "pkg", "facade"],
            "pub fn c() -> u64 { knot::k() + bin::b() + pkg::p() + facade::o_0() }\n",
        );
    }
    skew.add_rust(
        "extra_a",
        &["bin", "pkg", "facade"],
        "pub fn a() -> u64 { bin::b() + pkg::p() + facade::o_0() }\n",
    );
    skew.add_rust(
        "extra_b",
        &["bin", "pkg", "facade"],
        "pub fn b() -> u64 { bin::b() + pkg::p() + facade::o_0() }\n",
    );
    skew.add("src/bin/main.rs", "use crate::out_0;\nuse crate::out_1;\nuse crate::out_2;\nuse crate::out_3;\nuse crate::out_4;\nuse crate::out_5;\npub fn b() -> u64 { out_0::o_0() + out_1::o_1() + out_2::o_2() + out_3::o_3() + out_4::o_4() + out_5::o_5() }\n");
    skew.add("src/pkg/lib.rs", "use crate::out_0;\nuse crate::out_1;\nuse crate::out_2;\nuse crate::out_3;\nuse crate::out_4;\nuse crate::out_5;\npub fn p() -> u64 { out_0::o_0() + out_1::o_1() + out_2::o_2() + out_3::o_3() + out_4::o_4() + out_5::o_5() }\n");
    skew.add("src/facade/mod.rs", "pub use crate::out_0::o_0;\npub use crate::out_1::o_1;\npub use crate::out_2::o_2;\npub use crate::out_3::o_3;\npub use crate::out_4::o_4;\npub use crate::out_5::o_5;\n");

    let skew_report = skew.analyze(&cohesion_only_config())?;
    let skew_hits = diags_for(&skew_report, "architectural-crossing");
    assert_eq!(
        skew_hits.len(),
        1,
        "exempt nodes must not skew quantile to hide `knot`: {skew_hits:?}"
    );
    assert_eq!(
        skew_hits[0].span().file().as_path(),
        Path::new("src/knot.rs")
    );
    Ok(())
}

#[test]
fn architectural_crossing_counts_edges_from_and_to_exempt_nodes_in_fan_degrees()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    // Incoming from exempt node (`src/bin/main.rs`): 3 candidate callers + 1 exempt caller -> FanIn=4.
    let mut from_exempt = EdgeFixture::new();
    for i in 0..4 {
        from_exempt.add_rust(
            &format!("out_{i}"),
            &[],
            &format!("pub fn o_{i}() -> u64 {{ {i} }}\n"),
        );
        from_exempt.add_rust(
            &format!("pad_{i}"),
            &[],
            &format!("pub fn p_{i}() -> u64 {{ {i} }}\n"),
        );
    }
    from_exempt.add_rust(
        "hub",
        &["out_0", "out_1", "out_2", "out_3"],
        "pub fn h() -> u64 { out_0::o_0() + out_1::o_1() + out_2::o_2() + out_3::o_3() }\n",
    );
    for i in 0..3 {
        from_exempt.add_rust(
            &format!("in_{i}"),
            &["hub"],
            "pub fn c() -> u64 { hub::h() }\n",
        );
    }
    from_exempt.add(
        "src/bin/main.rs",
        "use crate::hub;\nfn main() { let _ = hub::h(); }\n",
    );
    assert_eq!(
        diags_for(&from_exempt.analyze(&cfg)?, "architectural-crossing").len(),
        1
    );

    // Outgoing to exempt node (`src/facade/mod.rs`): 3 candidate deps + 1 exempt dep -> FanOut=4.
    let mut to_exempt = EdgeFixture::new();
    for i in 0..3 {
        to_exempt.add_rust(
            &format!("out_{i}"),
            &[],
            &format!("pub fn o_{i}() -> u64 {{ {i} }}\n"),
        );
    }
    to_exempt.add("src/facade/mod.rs", "pub use crate::out_0::o_0;\n");
    to_exempt.add_rust(
        "hub",
        &["out_0", "out_1", "out_2", "facade"],
        "pub fn h() -> u64 { out_0::o_0() + out_1::o_1() + out_2::o_2() + facade::o_0() }\n",
    );
    for i in 0..4 {
        to_exempt.add_rust(
            &format!("in_{i}"),
            &["hub"],
            "pub fn c() -> u64 { hub::h() }\n",
        );
        to_exempt.add_rust(
            &format!("pad_{i}"),
            &[],
            &format!("pub fn p_{i}() -> u64 {{ {i} }}\n"),
        );
    }
    assert_eq!(
        diags_for(&to_exempt.analyze(&cfg)?, "architectural-crossing").len(),
        1
    );
    Ok(())
}

#[test]
fn architectural_crossing_deduplicates_edges_and_ignores_external_and_ts_unit_test_imports()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    // Case 1: Distinct-edge deduplication + unresolved std imports across multi-file module.
    // Note: `src/hub/mod.rs` places unresolved `std::` imports on lines 1-2 and its first
    // resolved internal import on line 3, verifying the crossing anchors at primary file line 1.
    let mut dedup = EdgeFixture::new();
    for i in 0..3 {
        dedup.add_rust(
            &format!("out_{i}"),
            &[],
            &format!("pub fn a() -> u64 {{ {i} }}\npub fn b() -> u64 {{ {i} + 1 }}\n"),
        );
    }
    dedup.add(
        "src/hub/mod.rs",
        "use std::collections::HashMap;\nuse std::path::PathBuf;\nuse crate::out_0::a as a0;\nuse crate::out_0::b as b0;\nuse crate::out_1::a as a1;\npub fn h0() -> u64 { let _ = (HashMap::<u8, u8>::new(), PathBuf::new()); a0() + b0() + a1() }\n",
    );
    dedup.add(
        "src/hub.rs",
        "use std::fmt::Display;\nuse crate::out_0::a as a0;\nuse crate::out_1::b as b1;\nuse crate::out_2::a as a2;\nuse crate::out_2::b as b2;\npub fn h1(_: &dyn Display) -> u64 { a0() + b1() + a2() + b2() }\n",
    );
    for i in 0..3 {
        dedup.add(
            format!("src/in_{i}.rs"),
            "use crate::hub::h0;\nuse crate::hub::h1;\npub fn c() -> u64 { h0() + h1(&1u64) }\n",
        );
    }
    for i in 0..5 {
        dedup.add_rust(
            &format!("pad_{i}"),
            &[],
            &format!("pub fn p() -> u64 {{ {i} }}\n"),
        );
    }
    assert_no_rule(&dedup.analyze(&cfg)?, "architectural-crossing");

    dedup.add_rust("out_3", &[], "pub fn a() -> u64 { 3 }\n");
    dedup.add_rust("in_3", &["hub"], "pub fn c3() -> u64 { hub::h0() }\n");
    for (path, content) in &mut dedup.files {
        if path == "src/hub.rs" {
            *content = format!("use crate::out_3;\n{content}");
        }
    }
    let dedup_report = dedup.analyze(&cfg)?;
    let dedup_hits = diags_for(&dedup_report, "architectural-crossing");
    assert_eq!(dedup_hits.len(), 1);
    assert_eq!(
        dedup_hits[0].span().file().as_path(),
        Path::new("src/hub/mod.rs")
    );
    assert_eq!(dedup_hits[0].span().start_line(), 1);

    // Case 2: TypeScript `UnitTest` files (`*.test.ts`) do not inflate FanIn.
    let mut ts_test_excl = EdgeFixture::new();
    for i in 0..4 {
        ts_test_excl.add_ts(
            &format!("out_{i}"),
            &[],
            &format!("export function sym(): number {{ return {i}; }}\n"),
        );
        ts_test_excl.add_ts(
            &format!("pad_{i}"),
            &[],
            &format!("export function sym(): number {{ return {i}; }}\n"),
        );
    }
    ts_test_excl.add_ts(
        "hub",
        &["./out_0", "./out_1", "./out_2", "./out_3"],
        "export function sym(): number { return s0() + s1() + s2() + s3(); }\n",
    );
    for i in 0..3 {
        ts_test_excl.add_ts(
            &format!("in_{i}"),
            &["./hub"],
            "export function c(): number { return s0(); }\n",
        );
    }
    ts_test_excl.add(
        "src/extra.test.ts",
        "import { sym } from \"./hub\";\ntest(\"h\", () => { expect(sym()).toBe(6); });\n",
    );
    assert_no_rule(&ts_test_excl.analyze(&cfg)?, "architectural-crossing");
    ts_test_excl.add_ts(
        "in_3",
        &["./hub"],
        "export function c3(): number { return s0(); }\n",
    );
    assert_eq!(
        diags_for(&ts_test_excl.analyze(&cfg)?, "architectural-crossing").len(),
        1
    );
    Ok(())
}

#[test]
fn architectural_crossing_ignores_ancestor_edges_integration_tests_and_cfg_test_imports()
-> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();

    // Case 1 (Incoming): `sub::child_*`, `tests/*.rs`, and `#[cfg(test)]` do not inflate FanIn.
    let mut incoming_proj = EdgeFixture::new();
    for i in 0..4 {
        incoming_proj.add_rust(
            &format!("out_{i}"),
            &[],
            &format!("pub fn o_{i}() -> u64 {{ {i} }}\n"),
        );
    }
    incoming_proj.add("src/sub/mod.rs", "use crate::out_0;\nuse crate::out_1;\nuse crate::out_2;\nuse crate::out_3;\npub fn sub_v() -> u64 { out_0::o_0() + out_1::o_1() + out_2::o_2() + out_3::o_3() }\n");
    incoming_proj.add(
        "src/sub/child_a.rs",
        "use crate::sub;\npub fn ca() -> u64 { sub::sub_v() }\n",
    );
    incoming_proj.add(
        "src/sub/child_b.rs",
        "use crate::sub;\npub fn cb() -> u64 { sub::sub_v() }\n",
    );
    for i in 0..3 {
        incoming_proj.add_rust(
            &format!("in_{i}"),
            &["sub"],
            "pub fn c() -> u64 { sub::sub_v() }\n",
        );
    }
    incoming_proj.add("src/pad_0.rs", "pub fn p0() -> u64 { 0 }\n#[cfg(test)]\nmod tests {\n    use crate::sub;\n    #[test]\n    fn t() { assert_eq!(sub::sub_v(), 6); }\n}\n");
    incoming_proj.add_rust("pad_1", &[], "pub fn p1() -> u64 { 1 }\n");
    incoming_proj.add(
        "tests/ext_a.rs",
        "use crate::sub;\n#[test] fn a() { let _ = sub::sub_v(); }\n",
    );
    incoming_proj.add(
        "tests/ext_b.rs",
        "use crate::sub;\n#[test] fn b() { let _ = sub::sub_v(); }\n",
    );
    assert_no_rule(&incoming_proj.analyze(&cfg)?, "architectural-crossing");

    incoming_proj.add_rust("in_3", &["sub"], "pub fn c3() -> u64 { sub::sub_v() }\n");
    assert_eq!(
        diags_for(&incoming_proj.analyze(&cfg)?, "architectural-crossing").len(),
        1
    );

    // Case 2 (Outgoing): child imports and `#[cfg(test)]` import do not inflate FanOut.
    let mut outgoing_proj = EdgeFixture::new();
    for i in 0..4 {
        outgoing_proj.add_rust(
            &format!("out_{i}"),
            &[],
            &format!("pub fn o_{i}() -> u64 {{ {i} }}\n"),
        );
    }
    outgoing_proj.add("src/sub/child_a.rs", "pub fn ca() -> u64 { 1 }\n");
    outgoing_proj.add("src/sub/child_b.rs", "pub fn cb() -> u64 { 2 }\n");
    outgoing_proj.add("src/sub/mod.rs", "use crate::out_0;\nuse crate::out_1;\nuse crate::out_2;\nuse crate::sub::child_a;\nuse crate::sub::child_b;\npub fn sub_v() -> u64 { out_0::o_0() + out_1::o_1() + out_2::o_2() + child_a::ca() + child_b::cb() }\n#[cfg(test)]\nmod tests {\n    use crate::out_3;\n    #[test]\n    fn t() { assert_eq!(out_3::o_3(), 3); }\n}\n");
    for i in 0..4 {
        outgoing_proj.add_rust(
            &format!("in_{i}"),
            &["sub"],
            "pub fn c() -> u64 { sub::sub_v() }\n",
        );
    }
    outgoing_proj.add_rust("pad_0", &[], "pub fn p0() -> u64 { 0 }\n");
    assert_no_rule(&outgoing_proj.analyze(&cfg)?, "architectural-crossing");
    for (path, content) in &mut outgoing_proj.files {
        if path == "src/sub/mod.rs" {
            *content = format!("use crate::out_3;\n{content}");
        }
    }
    assert_eq!(
        diags_for(&outgoing_proj.analyze(&cfg)?, "architectural-crossing").len(),
        1
    );
    Ok(())
}
