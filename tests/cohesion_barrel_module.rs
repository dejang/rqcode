mod support;

use rqcode::config::Config;
use rqcode::diagnostic::Severity;
use std::error::Error;
use support::{analyze_project, assert_no_rule, cohesion_only_config, diags_for};

type ProjectFiles<'a> = &'a [(&'a str, &'a str)];

fn boundaries_toml(extra: &str) -> Result<Config, Box<dyn Error>> {
    let toml = format!(
        "[similarity]\nenabled = false\n[test_coupling]\nenabled = false\n[ast_grep]\nenabled = false\nbuiltin_rules = false\n[cohesion.boundaries]\n{extra}\n"
    );
    Ok(Config::from_toml(&toml)?)
}

fn assert_single_barrel(
    label: &str,
    expected_file: &str,
    expected_line: usize,
    files: ProjectFiles<'_>,
) -> Result<(), Box<dyn Error>> {
    let diags = analyze_project(files, &cohesion_only_config())?;
    let hits = diags_for(&diags, "no-barrel-module");
    assert_eq!(
        hits.len(),
        1,
        "{label}: expected 1 diagnostic, got: {diags:?}"
    );
    assert_eq!(
        hits[0].severity(),
        Severity::Error,
        "{label}: must be Error"
    );
    assert!(
        hits[0].span().file().ends_with(expected_file),
        "{label}: wrong file {:?}",
        hits[0].span()
    );
    assert_eq!(
        hits[0].span().start_line(),
        expected_line,
        "{label}: wrong line {:?}",
        hits[0].span()
    );
    assert_eq!(
        hits[0].related().len(),
        0,
        "{label}: expected 0 related spans: {:?}",
        hits[0].related()
    );
    let suggestion = hits[0].suggestion().unwrap_or("");
    assert!(
        suggestion.to_ascii_lowercase().contains("direct"),
        "{label}: suggestion should mention direct imports, got: {suggestion:?}"
    );
    Ok(())
}

#[test]
fn no_barrel_module_flags_ts_directory_index_reexports_and_anchor_spans()
-> Result<(), Box<dyn Error>> {
    let order = (
        "src/models/order.ts",
        "export class Order { id: number = 1; }\n",
    );
    let def_order = ("src/models/order.ts", "export default class Order {}\n");
    for (label, target, line, barrel_src) in [
        (
            "direct internal wildcard",
            "src/models/index.ts",
            2,
            "// re-export\nexport * from './order';\n",
        ),
        (
            "direct internal namespace",
            "src/models/index.ts",
            2,
            "// ns\nexport * as OrderMod from './order';\n",
        ),
        (
            "local export before re-export",
            "src/models/index.ts",
            3,
            "function f() { return 1; }\nexport { f };\nexport { Order } from './order';\n",
        ),
        (
            "direct external",
            "src/ext/index.ts",
            2,
            "// ext\nexport { Client } from 'external-sdk';\n",
        ),
        (
            "external imported binding",
            "src/ext/index.ts",
            2,
            "import { Client } from 'external-sdk';\nexport { Client };\n",
        ),
        (
            "internal imported named",
            "src/models/index.ts",
            2,
            "import { Order } from './order';\nexport { Order };\n",
        ),
        (
            "type-only re-export",
            "src/models/index.ts",
            2,
            "// types\nexport type { Order } from './order';\n",
        ),
    ] {
        assert_single_barrel(label, target, line, &[order, (target, barrel_src)])?;
    }
    assert_single_barrel(
        "internal imported default",
        "src/models/index.ts",
        2,
        &[
            def_order,
            (
                "src/models/index.ts",
                "import Order from './order';\nexport default Order;\n",
            ),
        ],
    )
}

#[test]
fn no_barrel_module_flags_ts_pure_reexport_and_type_aliased_forms() -> Result<(), Box<dyn Error>> {
    let order = ("src/models/order.ts", "export class Order {}\n");
    let def_order = ("src/models/order.ts", "export default class Order {}\n");
    for (label, target, line, src) in [
        (
            "direct internal pure",
            "src/models/all.ts",
            2,
            "// barrel\nexport * from './order';\n",
        ),
        (
            "direct external pure",
            "src/ext.ts",
            1,
            "export * from 'third-party-lib';\n",
        ),
        (
            "external imported binding pure",
            "src/ext_bind.ts",
            2,
            "import { Client } from 'external-sdk';\nexport { Client };\n",
        ),
        (
            "import-then-export named",
            "src/models/facade.ts",
            2,
            "import { Order } from './order';\nexport { Order };\n",
        ),
        (
            "export type from",
            "src/models/types_barrel.ts",
            1,
            "export type { Order } from './order';\n",
        ),
        (
            "import type then export type",
            "src/models/type_bind.ts",
            2,
            "import type { Order } from './order';\nexport type { Order };\n",
        ),
        (
            "aliased import then aliased export",
            "src/models/double_alias.ts",
            2,
            "import { Order as A } from './order';\nexport { A as B };\n",
        ),
        (
            "namespace import then export",
            "src/models/ns_bind.ts",
            2,
            "import * as OrderNs from './order';\nexport { OrderNs };\n",
        ),
    ] {
        assert_single_barrel(label, target, line, &[order, (target, src)])?;
    }
    for (label, target, line, src) in [
        (
            "import-then-export default",
            "src/models/def.ts",
            2,
            "import Order from './order';\nexport default Order;\n",
        ),
        (
            "export default as alias",
            "src/models/def_alias.ts",
            1,
            "export { default as RenamedOrder } from './order';\n",
        ),
    ] {
        assert_single_barrel(label, target, line, &[def_order, (target, src)])?;
    }
    let clean = analyze_project(
        &[("src/app/index.ts", "export class App {}\n")],
        &cohesion_only_config(),
    )?;
    assert_no_rule(&clean, "no-barrel-module");
    Ok(())
}

#[test]
fn no_barrel_module_deduplicates_and_caps_related_reexport_spans() -> Result<(), Box<dyn Error>> {
    let five_reexports: ProjectFiles<'_> = &[
        ("src/models/a.ts", "export class A {}\n"),
        ("src/models/b.ts", "export class B {}\n"),
        ("src/models/c.ts", "export class C {}\n"),
        ("src/models/d.ts", "export class D {}\n"),
        ("src/models/e.ts", "export class E {}\n"),
        (
            "src/models/index.ts",
            "export { A } from './a';\nexport { B } from './b';\nexport { C } from './c';\nexport { D } from './d';\nexport { E } from './e';\n",
        ),
    ];
    let default_diags = analyze_project(five_reexports, &cohesion_only_config())?;
    let default_hits = diags_for(&default_diags, "no-barrel-module");
    assert_eq!(default_hits.len(), 1);
    assert_eq!(default_hits[0].span().start_line(), 1);
    let default_lines: Vec<usize> = default_hits[0]
        .related()
        .iter()
        .map(|r| r.span().start_line())
        .collect();
    assert_eq!(
        default_lines,
        vec![2, 3, 4],
        "related spans must be additional lines 2..=4"
    );

    let cap_one = analyze_project(five_reexports, &boundaries_toml("max_barrel_related = 1")?)?;
    let cap_one_lines: Vec<usize> = diags_for(&cap_one, "no-barrel-module")[0]
        .related()
        .iter()
        .map(|r| r.span().start_line())
        .collect();
    assert_eq!(cap_one_lines, vec![2]);

    let cap_zero = analyze_project(five_reexports, &boundaries_toml("max_barrel_related = 0")?)?;
    assert_eq!(
        diags_for(&cap_zero, "no-barrel-module")[0].related().len(),
        0
    );

    let multi_stmt: ProjectFiles<'_> = &[
        (
            "src/models/x.ts",
            "export class A {}\nexport class B {}\nexport class C {}\nexport class D {}\n",
        ),
        ("src/models/y.ts", "export class E {}\nexport class F {}\n"),
        ("src/models/z.ts", "export class G {}\n"),
        (
            "src/models/index.ts",
            "export { A, B, C, D } from './x';\nexport { E, F } from './y';\nexport { G } from './z';\n",
        ),
    ];
    let dedup_diags = analyze_project(multi_stmt, &boundaries_toml("max_barrel_related = 2")?)?;
    let dedup_hits = diags_for(&dedup_diags, "no-barrel-module");
    assert_eq!(dedup_hits.len(), 1);
    assert_eq!(dedup_hits[0].span().start_line(), 1);
    let dedup_lines: Vec<usize> = dedup_hits[0]
        .related()
        .iter()
        .map(|r| r.span().start_line())
        .collect();
    assert_eq!(
        dedup_lines,
        vec![2, 3],
        "must deduplicate multi-specifier spans before capping"
    );

    let single_multi = analyze_project(
        &[
            ("src/models/x.ts", "export class A {}\nexport class B {}\n"),
            ("src/models/index.ts", "export { A, B } from './x';\n"),
        ],
        &cohesion_only_config(),
    )?;
    assert_eq!(
        diags_for(&single_multi, "no-barrel-module")[0]
            .related()
            .len(),
        0
    );
    Ok(())
}
