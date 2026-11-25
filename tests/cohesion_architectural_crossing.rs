mod support;

use rqcode::config::Config;
use rqcode::diagnostic::{Diagnostic, Severity};
use std::error::Error;
use std::fmt::Write as _;
use std::path::Path;
use support::{analyze_project, assert_no_rule, cohesion_only_config, diags_for};

struct ProjectBuilder {
    files: Vec<(String, String)>,
}

impl ProjectBuilder {
    fn new() -> Self {
        Self { files: Vec::new() }
    }

    fn add(&mut self, path: impl Into<String>, src: impl Into<String>) {
        self.files.push((path.into(), src.into()));
    }

    fn add_rust_module(&mut self, name: &str, imports: &[&str], body: &str) {
        let mut src = String::new();
        for dep in imports {
            let _ = writeln!(src, "use crate::{dep};");
        }
        src.push_str(body);
        self.add(format!("src/{name}.rs"), src);
    }

    fn add_ts_module(&mut self, name: &str, imports: &[&str], body: &str) {
        let mut src = String::new();
        for (idx, dep) in imports.iter().enumerate() {
            let _ = writeln!(src, "import {{ sym as s{idx} }} from \"{dep}\";");
        }
        src.push_str(body);
        self.add(format!("src/{name}.ts"), src);
    }

    fn add_swift_target(&mut self, target: &str, imports: &[&str], body: &str) {
        let mut src = String::new();
        for dep in imports {
            let _ = writeln!(src, "import {dep}");
        }
        src.push_str(body);
        self.add(format!("Sources/{target}/{target}.swift"), src);
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

fn crossing_config(
    cohesion_section: &str,
    boundaries_section: &str,
    crossing_section: &str,
) -> Result<Config, Box<dyn Error>> {
    let toml = format!(
        "[similarity]\nenabled = false\n[test_coupling]\nenabled = false\n[ast_grep]\nenabled = false\nbuiltin_rules = false\n[cohesion]\n{cohesion_section}\n[cohesion.boundaries]\n{boundaries_section}\n[cohesion.crossing]\n{crossing_section}\n"
    );
    Ok(Config::from_toml(&toml)?)
}

fn build_rust_five_by_four_knot(split_hub_files: bool) -> ProjectBuilder {
    let mut builder = ProjectBuilder::new();
    for i in 0..4 {
        let out_name = format!("out_{i}");
        builder.add_rust_module(
            &out_name,
            &[],
            &format!("pub fn val_{i}() -> u64 {{ {i} }}\n"),
        );
    }
    if split_hub_files {
        builder.add(
            "src/hub/mod.rs",
            "use crate::out_0;\nuse crate::out_1;\nuse crate::out_2;\npub fn hub_primary() -> u64 { out_0::val_0() + out_1::val_1() + out_2::val_2() }\n",
        );
        builder.add(
            "src/hub.rs",
            "use crate::out_3;\npub fn hub_secondary() -> u64 { out_3::val_3() }\n",
        );
    } else {
        builder.add_rust_module(
            "hub",
            &["out_0", "out_1", "out_2", "out_3"],
            "pub fn hub_primary() -> u64 { out_0::val_0() + out_1::val_1() + out_2::val_2() + out_3::val_3() }\n",
        );
    }
    for i in 0..5 {
        let in_name = format!("in_{i}");
        builder.add_rust_module(
            &in_name,
            &["hub"],
            &format!("pub fn call_{i}() -> u64 {{ hub::hub_primary() + {i} }}\n"),
        );
    }
    builder.add_rust_module("pad_0", &[], "pub fn pad0() -> u64 { 1 }\n");
    builder.add_rust_module("pad_1", &[], "pub fn pad1() -> u64 { 2 }\n");
    builder
}

#[test]
fn architectural_crossing_flags_rust_with_primary_file_and_related_edge_caps()
-> Result<(), Box<dyn Error>> {
    let default_cfg = cohesion_only_config();

    // Rust: 12 candidate modules; `crate::hub` spans `src/hub/mod.rs` and `src/hub.rs`
    // (`src/hub/mod.rs` sorts before `src/hub.rs` component-wise) with unequal FanIn=5, FanOut=4.
    let rust_proj = build_rust_five_by_four_knot(true);
    let rust_report = rust_proj.analyze(&default_cfg)?;
    let rust_hits = diags_for(&rust_report, "architectural-crossing");
    assert_eq!(
        rust_hits.len(),
        1,
        "expected 1 Rust crossing: {rust_hits:?}"
    );
    let rust_diag = rust_hits[0];
    assert_eq!(rust_diag.severity(), Severity::Warning);
    assert_eq!(
        rust_diag.span().file().as_path(),
        Path::new("src/hub/mod.rs")
    );
    assert_eq!(rust_diag.span().start_line(), 1);
    assert!(
        rust_diag.message().contains("crate::hub")
            && rust_diag.message().contains("fan-in 5")
            && rust_diag.message().contains("fan-out 4"),
        "expected module path, fan-in 5, and fan-out 4 in message: {}",
        rust_diag.message()
    );
    assert!(rust_diag.suggestion().is_some_and(|s| !s.is_empty()));
    assert_eq!(
        rust_diag.related().len(),
        6,
        "default max_related_edges=3 should cap at 3 incoming + 3 outgoing: {:?}",
        rust_diag.related()
    );
    let incoming_files: Vec<&Path> = rust_diag
        .related()
        .iter()
        .filter(|r| r.span().file().to_string_lossy().starts_with("src/in_"))
        .map(|r| r.span().file().as_path())
        .collect();
    assert_eq!(
        incoming_files,
        vec![
            Path::new("src/in_0.rs"),
            Path::new("src/in_1.rs"),
            Path::new("src/in_2.rs"),
        ],
        "incoming related edges must be deterministically ordered by source path"
    );
    let outgoing_lines: Vec<(&Path, usize)> = rust_diag
        .related()
        .iter()
        .filter(|r| {
            let file = r.span().file().as_path();
            file == Path::new("src/hub/mod.rs") || file == Path::new("src/hub.rs")
        })
        .map(|r| (r.span().file().as_path(), r.span().start_line()))
        .collect();
    assert_eq!(
        outgoing_lines,
        vec![
            (Path::new("src/hub/mod.rs"), 1),
            (Path::new("src/hub/mod.rs"), 2),
            (Path::new("src/hub/mod.rs"), 3),
        ],
        "outgoing related edges must point to the first 3 representative imports"
    );

    // Configurable max_related_edges: 2 -> 4, 0 -> 0, 10 (exceeding degrees) -> 5 + 4 = 9.
    let cap_two_cfg = crossing_config("", "", "max_related_edges = 2")?;
    let cap_two_report = rust_proj.analyze(&cap_two_cfg)?;
    assert_eq!(
        diags_for(&cap_two_report, "architectural-crossing")[0]
            .related()
            .len(),
        4
    );

    let cap_zero_cfg = crossing_config("", "", "max_related_edges = 0")?;
    let cap_zero_report = rust_proj.analyze(&cap_zero_cfg)?;
    assert_eq!(
        diags_for(&cap_zero_report, "architectural-crossing")[0]
            .related()
            .len(),
        0
    );

    let cap_large_cfg = crossing_config("", "", "max_related_edges = 10")?;
    let cap_large_report = rust_proj.analyze(&cap_large_cfg)?;
    assert_eq!(
        diags_for(&cap_large_report, "architectural-crossing")[0]
            .related()
            .len(),
        9
    );
    Ok(())
}

#[test]
fn architectural_crossing_flags_typescript_and_swift_modules() -> Result<(), Box<dyn Error>> {
    let default_cfg = cohesion_only_config();

    // TypeScript: 12 candidate modules, FanIn=4, FanOut=4 on `src/hub.ts`.
    // Include `src/hub.test.ts` (which shares `src::hub` and sorts lexicographically before
    // `src/hub.ts`) to confirm primary-file selection ignores `UnitTest` files.
    let mut ts_proj = ProjectBuilder::new();
    for i in 0..4 {
        ts_proj.add_ts_module(
            &format!("out_{i}"),
            &[],
            &format!("export function sym(): number {{ return {i}; }}\n"),
        );
    }
    ts_proj.add_ts_module(
        "hub",
        &["./out_0", "./out_1", "./out_2", "./out_3"],
        "export function sym(): number { return s0() + s1() + s2() + s3(); }\n",
    );
    ts_proj.add(
        "src/hub.test.ts",
        "import { sym } from \"./hub\";\nimport { sym as p0 } from \"./pad_0\";\ntest(\"hub\", () => { expect(sym() + p0()).toBe(6); });\n",
    );
    for i in 0..4 {
        ts_proj.add_ts_module(
            &format!("in_{i}"),
            &["./hub"],
            &format!("export function call(): number {{ return s0() + {i}; }}\n"),
        );
    }
    for i in 0..3 {
        ts_proj.add_ts_module(
            &format!("pad_{i}"),
            &[],
            &format!("export function sym(): number {{ return {i}; }}\n"),
        );
    }
    let ts_report = ts_proj.analyze(&default_cfg)?;
    let ts_hits = diags_for(&ts_report, "architectural-crossing");
    assert_eq!(ts_hits.len(), 1, "expected 1 TS crossing: {ts_hits:?}");
    assert_eq!(ts_hits[0].severity(), Severity::Warning);
    assert_eq!(ts_hits[0].span().file().as_path(), Path::new("src/hub.ts"));
    assert_eq!(ts_hits[0].related().len(), 6);

    // Swift: 12 candidate modules under `Sources/`, FanIn=4, FanOut=4 on `Sources/Hub/Hub.swift`.
    let mut swift_proj = ProjectBuilder::new();
    for i in 0..4 {
        swift_proj.add_swift_target(
            &format!("Out{i}"),
            &[],
            &format!("public func outVal{i}() -> Int {{ return {i} }}\n"),
        );
    }
    swift_proj.add_swift_target(
        "Hub",
        &["Out0", "Out1", "Out2", "Out3"],
        "public func hubVal() -> Int { return outVal0() + outVal1() + outVal2() + outVal3() }\n",
    );
    for i in 0..4 {
        swift_proj.add_swift_target(
            &format!("In{i}"),
            &["Hub"],
            &format!("public func inVal{i}() -> Int {{ return hubVal() + {i} }}\n"),
        );
    }
    for i in 0..3 {
        swift_proj.add_swift_target(
            &format!("Pad{i}"),
            &[],
            &format!("public func padVal{i}() -> Int {{ return {i} }}\n"),
        );
    }
    let swift_report = swift_proj.analyze(&default_cfg)?;
    let swift_hits = diags_for(&swift_report, "architectural-crossing");
    assert_eq!(
        swift_hits.len(),
        1,
        "expected 1 Swift crossing: {swift_hits:?}"
    );
    assert_eq!(swift_hits[0].severity(), Severity::Warning);
    assert_eq!(
        swift_hits[0].span().file().as_path(),
        Path::new("Sources/Hub/Hub.swift")
    );
    assert_eq!(swift_hits[0].related().len(), 6);
    Ok(())
}

#[test]
fn architectural_crossing_ignores_leaf_utilities_orchestrators_and_sub_threshold_nodes()
-> Result<(), Box<dyn Error>> {
    // 12-module acyclic Rust graph containing:
    // - `leaf`: FanIn=5, FanOut=0 (foundational utility -> silent)
    // - `orchestrator`: FanIn=0, FanOut=5 (top-level coordinator -> silent)
    // - `almost_in`: FanIn=3, FanOut=5 (below min_fan_in=4 -> silent)
    // - `almost_out`: FanIn=5, FanOut=3 (below min_fan_out=4 -> silent)
    // - `knot`: FanIn=4, FanOut=4 (firing control -> flags!)
    // Sorted FanIn across 12 nodes:  [0,0,0,0,0,3,3,3,3,4,5,5] -> sorted[9] = 4 -> tau_in = 4.
    // Sorted FanOut across 12 nodes: [0,0,0,0,0,2,3,3,4,4,5,5] -> sorted[9] = 4 -> tau_out = 4.
    let mut proj = ProjectBuilder::new();
    proj.add_rust_module("leaf", &[], "pub fn leaf_v() -> u64 { 1 }\n");
    proj.add_rust_module("d0", &[], "pub fn d0_v() -> u64 { 1 }\n");
    proj.add_rust_module("d1", &[], "pub fn d1_v() -> u64 { 2 }\n");
    proj.add_rust_module("d2", &[], "pub fn d2_v() -> u64 { 3 }\n");
    proj.add_rust_module(
        "almost_out",
        &["leaf", "d0", "d1"],
        "pub fn ao_v() -> u64 { leaf::leaf_v() + d0::d0_v() + d1::d1_v() }\n",
    );
    proj.add_rust_module(
        "knot",
        &["leaf", "d0", "d1", "d2"],
        "pub fn knot_v() -> u64 { leaf::leaf_v() + d0::d0_v() + d1::d1_v() + d2::d2_v() }\n",
    );
    proj.add_rust_module(
        "almost_in",
        &["leaf", "d0", "d1", "d2", "almost_out"],
        "pub fn ai_v() -> u64 { leaf::leaf_v() + d0::d0_v() + d1::d1_v() + d2::d2_v() + almost_out::ao_v() }\n",
    );
    proj.add_rust_module(
        "orchestrator",
        &["leaf", "almost_out", "knot", "almost_in", "d2"],
        "pub fn orch_v() -> u64 { leaf::leaf_v() + almost_out::ao_v() + knot::knot_v() + almost_in::ai_v() + d2::d2_v() }\n",
    );
    proj.add_rust_module(
        "c0",
        &["leaf", "almost_out", "knot", "almost_in"],
        "pub fn c0_v() -> u64 { leaf::leaf_v() + almost_out::ao_v() + knot::knot_v() + almost_in::ai_v() }\n",
    );
    proj.add_rust_module(
        "c1",
        &["almost_out", "knot", "almost_in"],
        "pub fn c1_v() -> u64 { almost_out::ao_v() + knot::knot_v() + almost_in::ai_v() }\n",
    );
    proj.add_rust_module(
        "c2",
        &["almost_out", "knot"],
        "pub fn c2_v() -> u64 { almost_out::ao_v() + knot::knot_v() }\n",
    );
    proj.add_rust_module("c3", &[], "pub fn c3_v() -> u64 { 9 }\n");

    let report = proj.analyze(&cohesion_only_config())?;
    let hits = diags_for(&report, "architectural-crossing");
    assert_eq!(hits.len(), 1, "only `knot` should fire: {hits:?}");
    assert_eq!(hits[0].span().file().as_path(), Path::new("src/knot.rs"));
    Ok(())
}

#[test]
fn architectural_crossing_respects_severity_toggles_cycles_and_suppressions()
-> Result<(), Box<dyn Error>> {
    let proj = build_rust_five_by_four_knot(false);

    // 1. advisory_severity = "error" escalates crossing severity.
    let err_cfg = crossing_config("advisory_severity = \"error\"", "", "")?;
    let err_report = proj.analyze(&err_cfg)?;
    let err_hits = diags_for(&err_report, "architectural-crossing");
    assert_eq!(err_hits.len(), 1);
    assert_eq!(err_hits[0].severity(), Severity::Error);

    // 2. Disabling `flag_cyclic_modules` must NOT suppress `architectural-crossing`.
    let no_cycles_cfg = crossing_config("", "flag_cyclic_modules = false", "enabled = true")?;
    let no_cycles_report = proj.analyze(&no_cycles_cfg)?;
    assert_eq!(
        diags_for(&no_cycles_report, "architectural-crossing").len(),
        1,
        "ModuleGraph must still build for crossings when cycle detection is disabled"
    );

    // 3. Disabling `cohesion.crossing.enabled` or `cohesion.enabled` silences crossing diagnostics.
    let crossing_off = crossing_config("", "", "enabled = false")?;
    assert_no_rule(&proj.analyze(&crossing_off)?, "architectural-crossing");
    let cohesion_off = crossing_config("enabled = false", "", "enabled = true")?;
    assert_no_rule(&proj.analyze(&cohesion_off)?, "architectural-crossing");

    // 4. Hub participating in a 2-cycle emits BOTH `cyclic-module-dependency` and `architectural-crossing`.
    let mut cyclic = build_rust_five_by_four_knot(false);
    for (path, content) in &mut cyclic.files {
        if path == "src/out_0.rs" {
            *content = format!("use crate::hub;\n{content}");
        }
    }
    let cyclic_report = cyclic.analyze(&cohesion_only_config())?;
    assert_eq!(
        diags_for(&cyclic_report, "cyclic-module-dependency").len(),
        1
    );
    assert_eq!(diags_for(&cyclic_report, "architectural-crossing").len(), 1);

    // 5. File-level and line-1 inline suppressions both silence `architectural-crossing`.
    let mut suppressed_file = build_rust_five_by_four_knot(false);
    for (path, content) in &mut suppressed_file.files {
        if path == "src/hub.rs" {
            *content = format!("// rqcode:allow-file(architectural-crossing)\n{content}");
        }
    }
    assert_no_rule(
        &suppressed_file.analyze(&cohesion_only_config())?,
        "architectural-crossing",
    );

    let mut suppressed_line = build_rust_five_by_four_knot(false);
    for (path, content) in &mut suppressed_line.files {
        if path == "src/hub.rs" {
            *content = format!("// rqcode:allow(architectural-crossing)\n{content}");
        }
    }
    assert_no_rule(
        &suppressed_line.analyze(&cohesion_only_config())?,
        "architectural-crossing",
    );
    Ok(())
}
