mod support;

use rqcode::config::Config;
use rqcode::diagnostic::Diagnostic;
use std::error::Error;
use std::fmt::Write as _;
use std::path::Path;
use support::{analyze_project, assert_no_rule, cohesion_only_config, diags_for};

struct GraphFixture {
    files: Vec<(String, String)>,
}

impl GraphFixture {
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

fn crossing_only_config(crossing_section: &str) -> Result<Config, Box<dyn Error>> {
    let toml = format!(
        "[similarity]\nenabled = false\n[test_coupling]\nenabled = false\n[ast_grep]\nenabled = false\nbuiltin_rules = false\n[cohesion]\nenabled = true\n[cohesion.crossing]\n{crossing_section}\n"
    );
    Ok(Config::from_toml(&toml)?)
}

#[test]
fn architectural_crossing_enforces_min_graph_modules_and_handles_empty_candidate_partition()
-> Result<(), Box<dyn Error>> {
    let mut proj = GraphFixture::new();
    for i in 0..5 {
        proj.add_rust(
            &format!("out_{i}"),
            &[],
            &format!("pub fn o_{i}() -> u64 {{ {i} }}\n"),
        );
    }
    proj.add_rust(
        "hub",
        &["out_0", "out_1", "out_2", "out_3", "out_4"],
        "pub fn h() -> u64 { out_0::o_0() + out_1::o_1() + out_2::o_2() + out_3::o_3() + out_4::o_4() }\n",
    );
    for i in 0..5 {
        proj.add_rust(
            &format!("in_{i}"),
            &["hub"],
            "pub fn c() -> u64 { hub::h() }\n",
        );
    }
    proj.add("src/lib.rs", "pub mod hub;\n");
    proj.add(
        "src/bin/main.rs",
        "use crate::hub;\nfn main() { let _ = hub::h(); }\n",
    );
    proj.add("src/facade/mod.rs", "pub use crate::hub::h;\n");
    proj.add(
        "tests/integration.rs",
        "use crate::hub;\n#[test] fn t() { assert_eq!(hub::h(), 10); }\n",
    );

    let default_cfg = cohesion_only_config();
    assert_no_rule(&proj.analyze(&default_cfg)?, "architectural-crossing");

    let min_11_cfg = crossing_only_config("min_graph_modules = 11")?;
    let min_11_report = proj.analyze(&min_11_cfg)?;
    let min_11_hits = diags_for(&min_11_report, "architectural-crossing");
    assert_eq!(min_11_hits.len(), 1);
    assert_eq!(
        min_11_hits[0].span().file().as_path(),
        Path::new("src/hub.rs")
    );

    proj.add_rust("pad_12", &[], "pub fn extra() -> u64 { 1 }\n");
    let min_12_report = proj.analyze(&default_cfg)?;
    let min_12_hits = diags_for(&min_12_report, "architectural-crossing");
    assert_eq!(min_12_hits.len(), 1);
    assert_eq!(
        min_12_hits[0].span().file().as_path(),
        Path::new("src/hub.rs")
    );

    // All-exempt partition with min_graph_modules = 0 must not underflow `n - 1` or panic.
    let mut all_exempt = GraphFixture::new();
    all_exempt.add("src/lib.rs", "pub fn root_fn() -> u64 { 1 }\n");
    all_exempt.add("src/bin/main.rs", "fn main() {}\n");
    let min_zero_cfg = crossing_only_config("min_graph_modules = 0")?;
    assert_no_rule(
        &all_exempt.analyze(&min_zero_cfg)?,
        "architectural-crossing",
    );
    Ok(())
}

#[test]
fn architectural_crossing_partitions_size_gate_strictly_by_language() -> Result<(), Box<dyn Error>>
{
    let mut proj = GraphFixture::new();
    for i in 0..4 {
        proj.add_rust(
            &format!("out_{i}"),
            &[],
            &format!("pub fn o_{i}() -> u64 {{ {i} }}\n"),
        );
        proj.add_ts(
            &format!("out_{i}"),
            &[],
            &format!("export function sym(): number {{ return {i}; }}\n"),
        );
    }
    proj.add_rust(
        "hub",
        &["out_0", "out_1", "out_2", "out_3"],
        "pub fn h() -> u64 { out_0::o_0() + out_1::o_1() + out_2::o_2() + out_3::o_3() }\n",
    );
    proj.add_ts(
        "hub",
        &["./out_0", "./out_1", "./out_2", "./out_3"],
        "export function sym(): number { return s0() + s1() + s2() + s3(); }\n",
    );
    for i in 0..4 {
        proj.add_rust(
            &format!("in_{i}"),
            &["hub"],
            "pub fn c() -> u64 { hub::h() }\n",
        );
        proj.add_ts(
            &format!("in_{i}"),
            &["./hub"],
            "export function c(): number { return s0(); }\n",
        );
    }

    let cfg = cohesion_only_config();
    assert_no_rule(&proj.analyze(&cfg)?, "architectural-crossing");

    for i in 0..3 {
        proj.add_rust(
            &format!("pad_{i}"),
            &[],
            &format!("pub fn p_{i}() -> u64 {{ {i} }}\n"),
        );
    }
    let report = proj.analyze(&cfg)?;
    let hits = diags_for(&report, "architectural-crossing");
    assert_eq!(
        hits.len(),
        1,
        "only Rust partition reached 12 candidates: {hits:?}"
    );
    assert_eq!(hits[0].span().file().as_path(), Path::new("src/hub.rs"));
    Ok(())
}

#[test]
fn architectural_crossing_does_not_pool_quantiles_across_languages() -> Result<(), Box<dyn Error>> {
    // 12 Rust candidates (`hub` has (4,4), Rust tau=4) + 12 TS candidates in a 6x6 bipartite graph
    // (6 nodes with FanIn=6, 6 with FanOut=6, 0 TS crossings).
    // If pooled (n=24, p=20), pooled tau=6 would hide Rust `hub`; strict partitioning keeps `hub` firing.
    let mut dual = GraphFixture::new();
    for i in 0..4 {
        dual.add_rust(
            &format!("out_{i}"),
            &[],
            &format!("pub fn o_{i}() -> u64 {{ {i} }}\n"),
        );
        dual.add_rust(
            &format!("in_{i}"),
            &["hub"],
            "pub fn c() -> u64 { hub::h() }\n",
        );
    }
    dual.add_rust(
        "hub",
        &["out_0", "out_1", "out_2", "out_3"],
        "pub fn h() -> u64 { out_0::o_0() + out_1::o_1() + out_2::o_2() + out_3::o_3() }\n",
    );
    for i in 0..3 {
        dual.add_rust(
            &format!("pad_{i}"),
            &[],
            &format!("pub fn p_{i}() -> u64 {{ {i} }}\n"),
        );
    }
    for i in 0..6 {
        dual.add_ts(
            &format!("bot_{i}"),
            &[],
            &format!("export function sym(): number {{ return {i}; }}\n"),
        );
    }
    for i in 0..6 {
        dual.add_ts(
            &format!("top_{i}"),
            &[
                "./bot_0", "./bot_1", "./bot_2", "./bot_3", "./bot_4", "./bot_5",
            ],
            "export function run(): number { return s0() + s1() + s2() + s3() + s4() + s5(); }\n",
        );
    }
    let report = dual.analyze(&cohesion_only_config())?;
    let hits = diags_for(&report, "architectural-crossing");
    assert_eq!(
        hits.len(),
        1,
        "TS 6x6 partition must not pool quantiles with Rust: {hits:?}"
    );
    assert_eq!(hits[0].span().file().as_path(), Path::new("src/hub.rs"));
    Ok(())
}

#[test]
fn architectural_crossing_enforces_both_static_floors_independently() -> Result<(), Box<dyn Error>>
{
    // Part A: `min_fan_in` static floor (`sorted_in[9] = 1 < 4` on `[0x7, 1x4, 3]`).
    // Hub has (FanIn=3, FanOut=4): silent at default, fires at `min_fan_in=3`, silent when `min_fan_out=5`.
    let mut in_floor = GraphFixture::new();
    for i in 0..4 {
        in_floor.add_rust(
            &format!("out_{i}"),
            &[],
            &format!("pub fn o_{i}() -> u64 {{ {i} }}\n"),
        );
    }
    in_floor.add_rust(
        "hub",
        &["out_0", "out_1", "out_2", "out_3"],
        "pub fn h() -> u64 { out_0::o_0() + out_1::o_1() + out_2::o_2() + out_3::o_3() }\n",
    );
    for i in 0..3 {
        in_floor.add_rust(
            &format!("in_{i}"),
            &["hub"],
            "pub fn c() -> u64 { hub::h() }\n",
        );
    }
    for i in 0..4 {
        in_floor.add_rust(
            &format!("pad_{i}"),
            &[],
            &format!("pub fn p_{i}() -> u64 {{ {i} }}\n"),
        );
    }
    assert_no_rule(
        &in_floor.analyze(&cohesion_only_config())?,
        "architectural-crossing",
    );
    let min_in_3 = crossing_only_config("min_fan_in = 3")?;
    assert_eq!(
        diags_for(&in_floor.analyze(&min_in_3)?, "architectural-crossing").len(),
        1
    );
    let min_in_3_out_5 = crossing_only_config("min_fan_in = 3\nmin_fan_out = 5")?;
    assert_no_rule(
        &in_floor.analyze(&min_in_3_out_5)?,
        "architectural-crossing",
    );

    // Part B: `min_fan_out` static floor mirror. Hub has (FanIn=4, FanOut=3):
    // silent at default, fires at `min_fan_out=3`, silent when `min_fan_in=5`.
    let mut out_floor = GraphFixture::new();
    for i in 0..3 {
        out_floor.add_rust(
            &format!("out_{i}"),
            &[],
            &format!("pub fn o_{i}() -> u64 {{ {i} }}\n"),
        );
    }
    out_floor.add_rust(
        "hub",
        &["out_0", "out_1", "out_2"],
        "pub fn h() -> u64 { out_0::o_0() + out_1::o_1() + out_2::o_2() }\n",
    );
    for i in 0..4 {
        out_floor.add_rust(
            &format!("in_{i}"),
            &["hub"],
            "pub fn c() -> u64 { hub::h() }\n",
        );
    }
    for i in 0..4 {
        out_floor.add_rust(
            &format!("pad_{i}"),
            &[],
            &format!("pub fn p_{i}() -> u64 {{ {i} }}\n"),
        );
    }
    assert_no_rule(
        &out_floor.analyze(&cohesion_only_config())?,
        "architectural-crossing",
    );
    let min_out_3 = crossing_only_config("min_fan_out = 3")?;
    assert_eq!(
        diags_for(&out_floor.analyze(&min_out_3)?, "architectural-crossing").len(),
        1
    );
    let min_in_5_out_3 = crossing_only_config("min_fan_in = 5\nmin_fan_out = 3")?;
    assert_no_rule(
        &out_floor.analyze(&min_in_5_out_3)?,
        "architectural-crossing",
    );
    Ok(())
}

#[test]
fn architectural_crossing_enforces_asymmetric_in_and_out_quantile_arrays()
-> Result<(), Box<dyn Error>> {
    // Asymmetric 12-node DAG where `tau_in = 6 != tau_out = 4` (kills swapped arrays or
    // single-array reuse):
    // Sorted FanIn:  [0x6, 2, 2, 5, 6, 8, 8] -> sorted[9] = 6 -> tau_in = 6.
    // Sorted FanOut: [0x4, 3, 4x7]           -> sorted[9] = 4 -> tau_out = 4.
    // Only `k` (6, 4) fires; `l` (5, 4) is silent.
    let mut asym = GraphFixture::new();
    for i in 0..4 {
        asym.add_rust(
            &format!("b{i}"),
            &[],
            &format!("pub fn v() -> u64 {{ {i} }}\n"),
        );
    }
    asym.add_rust(
        "k",
        &["b0", "b1", "b2", "b3"],
        "pub fn v() -> u64 { b0::v() + b1::v() + b2::v() + b3::v() }\n",
    );
    asym.add_rust(
        "l",
        &["b0", "b1", "b2", "b3"],
        "pub fn v() -> u64 { b0::v() + b1::v() + b2::v() + b3::v() }\n",
    );
    asym.add_rust(
        "t0",
        &["k", "b0", "b1"],
        "pub fn v() -> u64 { k::v() + b0::v() + b1::v() }\n",
    );
    for i in 1..=5 {
        asym.add_rust(
            &format!("t{i}"),
            &["k", "l", "b0", "b1"],
            "pub fn v() -> u64 { k::v() + l::v() + b0::v() + b1::v() }\n",
        );
    }
    let asym_report = asym.analyze(&cohesion_only_config())?;
    let asym_hits = diags_for(&asym_report, "architectural-crossing");
    assert_eq!(
        asym_hits.len(),
        1,
        "only `k` (6,4) should fire at tau_in=6, tau_out=4: {asym_hits:?}"
    );
    assert_eq!(asym_hits[0].span().file().as_path(), Path::new("src/k.rs"));
    Ok(())
}

#[test]
fn architectural_crossing_uses_exact_floor_index_and_clamps_quantile() -> Result<(), Box<dyn Error>>
{
    // 12-candidate acyclic Rust DAG (`n00`..`n11`):
    // - FanIn  sorted: `[0, 0, 0, 0, 0, 3, 4, 4, 4, 5, 6, 7]` -> `sorted[9]=5`, `sorted[10]=6`, `sorted[11]=7`
    // - FanOut sorted: `[0, 0, 0, 0, 3, 3, 3, 3, 4, 5, 6, 6]` -> `sorted[9]=5`, `sorted[10]=6`, `sorted[11]=6`
    // At `q = 0.90`, `p = floor(0.90 * 11) = 9` -> `tau_in = 5, tau_out = 5`:
    // `n05` (5,6) and `n06` (6,5) fire; `n07` (7,4) is silent.
    let mut dag = GraphFixture::new();
    for i in 8..=11 {
        dag.add_rust(
            &format!("n{i:02}"),
            &[],
            &format!("pub fn v() -> u64 {{ {i} }}\n"),
        );
    }
    dag.add_rust(
        "n07",
        &["n08", "n09", "n10", "n11"],
        "pub fn v() -> u64 { n08::v() + n09::v() + n10::v() + n11::v() }\n",
    );
    dag.add_rust(
        "n06",
        &["n07", "n08", "n09", "n10", "n11"],
        "pub fn v() -> u64 { n07::v() + n08::v() + n09::v() + n10::v() + n11::v() }\n",
    );
    dag.add_rust(
        "n05",
        &["n06", "n07", "n08", "n09", "n10", "n11"],
        "pub fn v() -> u64 { n06::v() + n07::v() + n08::v() + n09::v() + n10::v() + n11::v() }\n",
    );
    dag.add_rust(
        "n04",
        &["n05", "n06", "n07", "n08", "n09", "n10"],
        "pub fn v() -> u64 { n05::v() + n06::v() + n07::v() + n08::v() + n09::v() + n10::v() }\n",
    );
    for idx in 0..4 {
        dag.add_rust(
            &format!("n0{idx}"),
            &["n05", "n06", "n07"],
            "pub fn v() -> u64 { n05::v() + n06::v() + n07::v() }\n",
        );
    }
    let default_report = dag.analyze(&cohesion_only_config())?;
    let flagged: Vec<&Path> = diags_for(&default_report, "architectural-crossing")
        .iter()
        .map(|d| d.span().file().as_path())
        .collect();
    assert_eq!(
        flagged,
        vec![Path::new("src/n05.rs"), Path::new("src/n06.rs")]
    );

    // Quantile clamping (`1.5`/`inf` -> 1.0, `-0.5`/`-inf` -> 0.0).
    for hi_q in ["quantile = 1.5", "quantile = inf"] {
        assert_no_rule(
            &dag.analyze(&crossing_only_config(hi_q)?)?,
            "architectural-crossing",
        );
    }
    let expected_lo = [
        Path::new("src/n05.rs"),
        Path::new("src/n06.rs"),
        Path::new("src/n07.rs"),
    ];
    for lo_q in ["quantile = -0.5", "quantile = -inf"] {
        let report = dag.analyze(&crossing_only_config(lo_q)?)?;
        let hits = diags_for(&report, "architectural-crossing");
        assert!(
            hits.iter()
                .map(|d| d.span().file().as_path())
                .eq(expected_lo.iter().copied()),
            "expected {expected_lo:?} for {lo_q}, got: {hits:?}"
        );
    }
    Ok(())
}
