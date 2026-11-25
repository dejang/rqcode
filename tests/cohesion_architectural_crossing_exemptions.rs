mod support;

use rqcode::config::Config;
use rqcode::diagnostic::Diagnostic;
use std::error::Error;
use std::fmt::Write as _;
use std::path::Path;
use support::{analyze_project, assert_no_rule, cohesion_only_config, diags_for};

struct ExemptionFixture {
    files: Vec<(String, String)>,
}

impl ExemptionFixture {
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

    fn add_swift(&mut self, target: &str, imports: &[&str], body: &str) {
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

#[test]
fn architectural_crossing_exempts_rust_entrypoints_library_roots_and_pure_facades()
-> Result<(), Box<dyn Error>> {
    // 12 regular candidate modules (5 outs + 5 ins + `real_knot` + `mixed`), where every middle
    // node has FanIn=5, FanOut=5 (`tau_in = 5, tau_out = 5`), PLUS 3 high-fan exempt nodes:
    // - `src/bin/main.rs` + `src/bin.rs` (ANY file is BinaryEntrypoint -> exempt)
    // - `src/pkg/lib.rs` + `src/pkg.rs` (ANY file is LibraryRoot -> exempt)
    // - `src/facade/mod.rs` (ALL files have 0 types and 0 functions -> exempt)
    // Meanwhile `crate::mixed` has empty `src/mixed/mod.rs` + types-only `src/mixed.rs` (0 funcs,
    // 1 struct) -> exercises both `all(...)` and `f.types.is_empty()` so `crate::mixed` MUST fire.
    let mut proj = ExemptionFixture::new();
    for i in 0..5 {
        proj.add_rust(
            &format!("out_{i}"),
            &[],
            &format!("pub fn val_{i}() -> u64 {{ {i} }}\n"),
        );
    }
    proj.add_rust(
        "real_knot",
        &["out_0", "out_1", "out_2", "out_3", "out_4"],
        "pub fn knot() -> u64 { out_0::val_0() + out_1::val_1() + out_2::val_2() + out_3::val_3() + out_4::val_4() }\n",
    );
    proj.add(
        "src/mixed/mod.rs",
        "pub use crate::out_0::val_0;\npub use crate::out_1::val_1;\n",
    );
    proj.add(
        "src/mixed.rs",
        "use crate::out_2;\nuse crate::out_3;\nuse crate::out_4;\npub struct MixedRecord { pub a: u64, pub b: u64 }\n",
    );

    // Exempt node 1: ANY file is BinaryEntrypoint (`src/bin/main.rs` + `src/bin.rs`).
    proj.add(
        "src/bin/main.rs",
        "use crate::out_0;\nuse crate::out_1;\npub fn run_bin() -> u64 { out_0::val_0() + out_1::val_1() }\n",
    );
    proj.add(
        "src/bin.rs",
        "use crate::out_2;\nuse crate::out_3;\nuse crate::out_4;\npub fn bin_extra() -> u64 { out_2::val_2() + out_3::val_3() + out_4::val_4() }\n",
    );
    // Exempt node 2: ANY file is LibraryRoot (`src/pkg/lib.rs` + `src/pkg.rs`).
    proj.add(
        "src/pkg/lib.rs",
        "use crate::out_0;\nuse crate::out_1;\npub fn pkg_root() -> u64 { out_0::val_0() + out_1::val_1() }\n",
    );
    proj.add(
        "src/pkg.rs",
        "use crate::out_2;\nuse crate::out_3;\nuse crate::out_4;\npub fn pkg_extra() -> u64 { out_2::val_2() + out_3::val_3() + out_4::val_4() }\n",
    );
    // Exempt node 3: ALL files in `crate::facade` define 0 types and 0 functions (pure `mod.rs` facade).
    proj.add(
        "src/facade/mod.rs",
        "pub use crate::out_0::val_0;\npub use crate::out_1::val_1;\npub use crate::out_2::val_2;\npub use crate::out_3::val_3;\npub use crate::out_4::val_4;\n",
    );

    for i in 0..5 {
        proj.add_rust(
            &format!("in_{i}"),
            &["real_knot", "mixed", "bin", "pkg", "facade"],
            &format!(
                "pub fn call_{i}(r: mixed::MixedRecord) -> u64 {{ real_knot::knot() + r.a + bin::run_bin() + pkg::pkg_root() + facade::val_0() }}\n"
            ),
        );
    }

    let report = proj.analyze(&cohesion_only_config())?;
    assert_no_rule(&report, "cyclic-module-dependency");
    let hits = diags_for(&report, "architectural-crossing");
    let flagged_files: Vec<&Path> = hits.iter().map(|d| d.span().file().as_path()).collect();
    assert_eq!(
        flagged_files,
        vec![Path::new("src/mixed/mod.rs"), Path::new("src/real_knot.rs")],
        "expected only `crate::mixed` and `crate::real_knot` to fire, exempting bin/pkg/facade: {hits:?}"
    );
    Ok(())
}

#[test]
fn architectural_crossing_exempts_ts_roots_cli_and_barrels_without_shielding_real_siblings()
-> Result<(), Box<dyn Error>> {
    // 17 TS candidate modules (7 outs + 7 ins + `real_knot` + `sibling` + `combo`), where every
    // middle node has FanIn=7, FanOut=7 (`p = floor(0.90*16) = 14 -> tau_in=7, tau_out=7`), PLUS
    // 4 high-fan exempt TS nodes:
    // - `index.ts` (LibraryRoot -> exempt)
    // - `src::cli` split into `src/cli/index.ts` (Module, BarrelKind::None, sorts FIRST) +
    //   `src/cli.ts` (BinaryEntrypoint, sorts SECOND) -> exercises `any(role)` over `files[0].role`
    // - `src/barrel/index.ts` (DirectoryIndex barrel WITH a local function + sibling `index.test.ts`
    //   -> isolates `all(is_barrel)` from `all(empty)` and verifies UnitTest does not un-exempt)
    // - `src/reexport.ts` (PureReexport barrel -> exempt)
    // Non-exempt controls:
    // - `src/real_knot.ts` (regular module)
    // - `src::sibling` (`src/sibling/index.ts` barrel + `src/sibling.ts` defining ONLY an `interface`)
    // - `src::combo` (`src/combo/index.ts` barrel WITH local function + `src/combo.ts` non-barrel
    //   with 0 types and 0 functions -> rules out per-file `all(is_barrel || empty)`)
    let mut proj = ExemptionFixture::new();
    for i in 0..7 {
        proj.add_ts(
            &format!("out_{i}"),
            &[],
            &format!("export function sym(): number {{ return {i}; }}\n"),
        );
    }
    proj.add_ts(
        "real_knot",
        &["./out_0", "./out_1", "./out_2", "./out_3", "./out_4", "./out_5", "./out_6"],
        "export function sym(): number { return s0() + s1() + s2() + s3() + s4() + s5() + s6(); }\n",
    );
    proj.add(
        "src/sibling/index.ts",
        "export { sym as b0 } from \"../out_0\";\nexport { sym as b1 } from \"../out_1\";\nexport { sym as b2 } from \"../out_2\";\n",
    );
    proj.add_ts(
        "sibling",
        &["./out_3", "./out_4", "./out_5", "./out_6"],
        "export interface SiblingSpec { count: number; }\n",
    );
    proj.add(
        "src/combo/index.ts",
        "export { sym as c0 } from \"../out_0\";\nexport { sym as c1 } from \"../out_1\";\nexport { sym as c2 } from \"../out_2\";\nexport function localCombo(): number { return 1; }\n",
    );
    proj.add_ts(
        "combo",
        &["./out_3", "./out_4", "./out_5", "./out_6"],
        "console.log(s0() + s1() + s2() + s3());\n",
    );

    proj.add(
        "index.ts",
        "import { sym as a } from \"./src/out_0\";\nimport { sym as b } from \"./src/out_1\";\nimport { sym as c } from \"./src/out_2\";\nimport { sym as d } from \"./src/out_3\";\nimport { sym as e } from \"./src/out_4\";\nimport { sym as f } from \"./src/out_5\";\nimport { sym as g } from \"./src/out_6\";\nexport function sym(): number { return a() + b() + c() + d() + e() + f() + g(); }\n",
    );
    proj.add(
        "src/cli/index.ts",
        "import { sym as s0 } from \"../out_0\";\nimport { sym as s1 } from \"../out_1\";\nimport { sym as s2 } from \"../out_2\";\nexport function sym(): number { return s0() + s1() + s2(); }\n",
    );
    proj.add_ts(
        "cli",
        &["./out_3", "./out_4", "./out_5", "./out_6"],
        "export function runCli(): number { return s0() + s1() + s2() + s3(); }\n",
    );
    proj.add(
        "src/barrel/index.ts",
        "export { sym as a } from \"../out_0\";\nexport { sym as b } from \"../out_1\";\nexport { sym as c } from \"../out_2\";\nexport { sym as d } from \"../out_3\";\nexport { sym as e } from \"../out_4\";\nexport { sym as f } from \"../out_5\";\nexport { sym as g } from \"../out_6\";\nexport function localHelper(): number { return 7; }\n",
    );
    proj.add(
        "src/barrel/index.test.ts",
        "import { localHelper } from \"./index\";\ntest(\"barrel\", () => { expect(localHelper()).toBe(7); });\n",
    );
    proj.add(
        "src/reexport.ts",
        "export { sym as a } from \"./out_0\";\nexport { sym as b } from \"./out_1\";\nexport { sym as c } from \"./out_2\";\nexport { sym as d } from \"./out_3\";\nexport { sym as e } from \"./out_4\";\nexport { sym as f } from \"./out_5\";\nexport { sym as g } from \"./out_6\";\n",
    );

    for i in 0..7 {
        proj.add_ts(
            &format!("in_{i}"),
            &[
                "./real_knot",
                "./sibling",
                "./combo",
                "../index",
                "./cli",
                "./barrel",
                "./reexport",
            ],
            &format!("export function run(): number {{ return s0() + s3() + s4() + {i}; }}\n"),
        );
    }

    let report = proj.analyze(&cohesion_only_config())?;
    assert_no_rule(&report, "cyclic-module-dependency");
    let barrel_files: Vec<&Path> = diags_for(&report, "no-barrel-module")
        .iter()
        .map(|d| d.span().file().as_path())
        .collect();
    assert!(
        barrel_files.contains(&Path::new("src/barrel/index.ts"))
            && barrel_files.contains(&Path::new("src/reexport.ts")),
        "pure barrels must still be flagged by no-barrel-module: {barrel_files:?}"
    );

    let crossing_hits = diags_for(&report, "architectural-crossing");
    let flagged_files: Vec<&Path> = crossing_hits
        .iter()
        .map(|d| d.span().file().as_path())
        .collect();
    assert_eq!(
        flagged_files,
        vec![
            Path::new("src/combo/index.ts"),
            Path::new("src/real_knot.ts"),
            Path::new("src/sibling/index.ts"),
        ],
        "expected only combo, real_knot, and sibling to fire as crossings: {crossing_hits:?}"
    );
    Ok(())
}

#[test]
fn architectural_crossing_exempts_swift_path_and_at_main_entrypoints() -> Result<(), Box<dyn Error>>
{
    // 12 regular Swift candidates (4 Outs + 4 Ins + RealKnot + 3 Pads) where RealKnot has
    // FanIn=4, FanOut=4 (`tau_in = 4, tau_out = 4`), PLUS 2 high-fan Swift BinaryEntrypoints:
    // - `Sources/main/main.swift` (classified by filename -> FileRole::BinaryEntrypoint)
    // - `Sources/AppEntry/AppEntry.swift` (classified by `@main` attribute in extractor -> FileRole::BinaryEntrypoint)
    let mut proj = ExemptionFixture::new();
    for i in 0..4 {
        proj.add_swift(
            &format!("Out{i}"),
            &[],
            &format!("public func outVal{i}() -> Int {{ return {i} }}\n"),
        );
    }
    proj.add_swift(
        "RealKnot",
        &["Out0", "Out1", "Out2", "Out3"],
        "public func knotVal() -> Int { return outVal0() + outVal1() + outVal2() + outVal3() }\n",
    );
    proj.add(
        "Sources/main/main.swift",
        "import Out0\nimport Out1\nimport Out2\nimport Out3\npublic func mainVal() -> Int { return outVal0() + outVal1() + outVal2() + outVal3() }\n",
    );
    proj.add_swift(
        "AppEntry",
        &["Out0", "Out1", "Out2", "Out3"],
        "@main\npublic struct AppEntry {\n    public static func main() { let _ = outVal0() + outVal1() + outVal2() + outVal3() }\n}\n",
    );
    for i in 0..4 {
        proj.add_swift(
            &format!("In{i}"),
            &["RealKnot", "main", "AppEntry"],
            &format!("public func inVal{i}() -> Int {{ return knotVal() + mainVal() + {i} }}\n"),
        );
    }
    for i in 0..3 {
        proj.add_swift(
            &format!("Pad{i}"),
            &[],
            &format!("public func padVal{i}() -> Int {{ return {i} }}\n"),
        );
    }

    let report = proj.analyze(&cohesion_only_config())?;
    assert_no_rule(&report, "cyclic-module-dependency");
    let hits = diags_for(&report, "architectural-crossing");
    assert_eq!(
        hits.len(),
        1,
        "only RealKnot should fire, exempting main.swift and @main: {hits:?}"
    );
    assert_eq!(
        hits[0].span().file().as_path(),
        Path::new("Sources/RealKnot/RealKnot.swift")
    );
    Ok(())
}
