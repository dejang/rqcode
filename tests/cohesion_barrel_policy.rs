mod support;

use rqcode::config::{Config, PublicApiScope};
use rqcode::diagnostic::Severity;
use std::error::Error;
use support::{analyze_project, assert_no_rule, cohesion_only_config, diags_for};

type ProjectFiles<'a> = &'a [(&'a str, &'a str)];

fn cohesion_toml(
    cohesion_section: &str,
    boundaries_section: &str,
) -> Result<Config, Box<dyn Error>> {
    let toml = format!(
        "[similarity]\nenabled = false\n[test_coupling]\nenabled = false\n[ast_grep]\nenabled = false\nbuiltin_rules = false\n[cohesion]\n{cohesion_section}\n[cohesion.boundaries]\n{boundaries_section}\n"
    );
    Ok(Config::from_toml(&toml)?)
}

fn test_coupling_all_config() -> Config {
    let mut cfg = Config::default();
    cfg.cohesion.enabled = false;
    cfg.similarity.enabled = false;
    cfg.test_coupling.enabled = true;
    cfg.test_coupling.public_api_scope = PublicApiScope::All;
    cfg.ast_grep.enabled = false;
    cfg.ast_grep.builtin_rules = false;
    cfg
}

#[test]
fn no_barrel_module_exempts_non_index_files_with_local_declarations() -> Result<(), Box<dyn Error>>
{
    let cfg = cohesion_only_config();
    let order = ("src/order.ts", "export class Order {}\n");
    for (label, path, src) in [
        (
            "exported type alias",
            "src/a.ts",
            "export { Order } from './order';\nexport type OrderId = string;\n",
        ),
        (
            "exported const",
            "src/b.ts",
            "export { Order } from './order';\nexport const MAX = 10;\n",
        ),
        (
            "exported function",
            "src/c.ts",
            "export { Order } from './order';\nexport function makeId() { return 1; }\n",
        ),
        (
            "unexported class",
            "src/d.ts",
            "export { Order } from './order';\nclass Local {}\n",
        ),
        (
            "unexported interface",
            "src/e.ts",
            "export { Order } from './order';\ninterface Local { x: number; }\n",
        ),
        (
            "unexported enum",
            "src/f.ts",
            "export { Order } from './order';\nenum Kind { A }\n",
        ),
        (
            "unexported type",
            "src/g.ts",
            "export { Order } from './order';\ntype LocalId = string;\n",
        ),
        (
            "unexported const",
            "src/h.ts",
            "export { Order } from './order';\nconst SECRET = 1;\n",
        ),
        (
            "unexported function",
            "src/i.ts",
            "export { Order } from './order';\nfunction helper() { return 1; }\n",
        ),
        (
            "expression statement",
            "src/j.ts",
            "export { Order } from './order';\nconsole.log('init');\n",
        ),
        (
            "local default value",
            "src/k.ts",
            "export { Order } from './order';\nexport default 42;\n",
        ),
    ] {
        let diags = analyze_project(&[order, (path, src)], &cfg)?;
        assert_eq!(
            diags_for(&diags, "no-barrel-module").len(),
            0,
            "{label}: got {diags:?}"
        );
    }
    Ok(())
}

#[test]
fn no_barrel_module_exempts_ambient_test_and_module_tree_files() -> Result<(), Box<dyn Error>> {
    let cfg = cohesion_only_config();
    let clean_files: ProjectFiles<'_> = &[
        ("src/order.ts", "export class Order {}\n"),
        (
            "src/app/index.ts",
            "export class App { run() { return 1; } }\n",
        ),
        (
            "src/local/index.ts",
            "function run() { return 1; }\nexport { run };\n",
        ),
        ("src/ambient/index.d.ts", "export * from './t';\n"),
        ("src/ambient/reexport.d.ts", "export * from './t';\n"),
        (
            "tests/helpers/index.ts",
            "export { Order } from '../../src/order';\n",
        ),
        ("src/app/index.test.ts", "export * from './index';\n"),
        ("src/lib.rs", "pub mod m;\npub use m::O;\n"),
        ("src/m/mod.rs", "pub use crate::O;\n"),
        ("src/prelude.rs", "pub use crate::O;\n"),
        (
            "Sources/Core/Exports.swift",
            "@_exported import Foundation\n",
        ),
    ];
    assert_no_rule(&analyze_project(clean_files, &cfg)?, "no-barrel-module");
    Ok(())
}

#[test]
fn no_barrel_module_respects_policy_severity_and_inline_suppressions() -> Result<(), Box<dyn Error>>
{
    let cfg = cohesion_only_config();
    let lib_root: ProjectFiles<'_> = &[
        ("src/order.ts", "export class Order {}\n"),
        ("src/index.ts", "export { Order } from './order';\n"),
        ("src/models/item.ts", "export class Item {}\n"),
        ("src/models/index.ts", "export { Item } from './item';\n"),
        ("src/public_api.ts", "export { Order } from './order';\n"),
        (
            "packages/a/src/index.ts",
            "export { Order } from '../../../src/order';\n",
        ),
    ];
    assert_eq!(
        diags_for(&analyze_project(lib_root, &cfg)?, "no-barrel-module").len(),
        4
    );

    let warn_diags = analyze_project(lib_root, &cohesion_toml("severity = \"warning\"", "")?)?;
    let warn_hits = diags_for(&warn_diags, "no-barrel-module");
    assert_eq!(warn_hits.len(), 4);
    assert!(warn_hits.iter().all(|d| d.severity() == Severity::Warning));

    let allow_root_cfg = cohesion_toml("", "allow_library_root_barrel = true")?;
    let allow_root_diags = analyze_project(lib_root, &allow_root_cfg)?;
    let allow_root_hits = diags_for(&allow_root_diags, "no-barrel-module");
    assert_eq!(
        allow_root_hits.len(),
        3,
        "only src/index.ts exempt: {allow_root_hits:?}"
    );
    assert!(
        allow_root_hits
            .iter()
            .all(|d| !d.span().file().ends_with("src/index.ts")
                || d.span().file().to_string_lossy().contains("packages"))
    );

    let root_idx: ProjectFiles<'_> = &[
        ("order.ts", "export class Order {}\n"),
        ("index.ts", "export { Order } from './order';\n"),
    ];
    assert_eq!(
        diags_for(&analyze_project(root_idx, &cfg)?, "no-barrel-module").len(),
        1
    );
    assert_no_rule(
        &analyze_project(root_idx, &allow_root_cfg)?,
        "no-barrel-module",
    );
    assert_no_rule(
        &analyze_project(lib_root, &cohesion_toml("", "flag_barrel_modules = false")?)?,
        "no-barrel-module",
    );
    assert_no_rule(
        &analyze_project(lib_root, &cohesion_toml("enabled = false", "")?)?,
        "no-barrel-module",
    );

    let order = ("src/models/order.ts", "export class Order {}\n");
    let user = ("src/models/user.ts", "export class User {}\n");
    assert_no_rule(
        &analyze_project(
            &[
                order,
                (
                    "src/models/index.ts",
                    "// rqcode:allow-file(no-barrel-module)\nexport { Order } from './order';\n",
                ),
            ],
            &cfg,
        )?,
        "no-barrel-module",
    );
    assert_no_rule(
        &analyze_project(
            &[
                order,
                (
                    "src/models/index.ts",
                    "// rqcode:allow(no-barrel-module)\nexport { Order } from './order';\n",
                ),
            ],
            &cfg,
        )?,
        "no-barrel-module",
    );
    let second_only = analyze_project(
        &[
            order,
            user,
            (
                "src/models/index.ts",
                "export { Order } from './order';\n\n// rqcode:allow(no-barrel-module)\nexport { User } from './user';\n",
            ),
        ],
        &cfg,
    )?;
    assert_eq!(diags_for(&second_only, "no-barrel-module").len(), 1);
    Ok(())
}

#[test]
fn barrel_public_surface_boundary_rules_by_kind_and_origin() -> Result<(), Box<dyn Error>> {
    let tc_cfg = test_coupling_all_config();

    for (label, index_src) in [
        ("direct external", "export * from 'external-sdk';\n"),
        (
            "external imported binding",
            "import { Client } from 'external-sdk';\nexport { Client };\n",
        ),
        ("path alias direct", "export { X } from '@/x';\n"),
        (
            "path alias imported binding",
            "import { X } from '@/x';\nexport { X };\n",
        ),
    ] {
        let diags = analyze_project(
            &[
                ("src/models/index.ts", index_src),
                (
                    "src/models/order.ts",
                    "export class Order { total(): number { return 42; } }\n",
                ),
                (
                    "src/order.test.ts",
                    "import { Order } from './models/order';\ntest('works', () => { expect(new Order().total()).toBe(42); });\n",
                ),
            ],
            &tc_cfg,
        )?;
        assert_eq!(
            diags_for(&diags, "test-bypasses-public-surface").len(),
            0,
            "{label}: {diags:?}"
        );
    }

    for (label, index_src) in [
        (
            "direct internal",
            "export { PublicOrder } from './public';\n",
        ),
        (
            "internal imported binding",
            "import { PublicOrder } from './public';\nexport { PublicOrder };\n",
        ),
        (
            "mixed external then internal",
            "export * from 'external-sdk';\nexport { PublicOrder } from './public';\n",
        ),
    ] {
        let diags = analyze_project(
            &[
                ("src/models/public.ts", "export class PublicOrder {}\n"),
                (
                    "src/models/secret.ts",
                    "export class SecretHelper { run(): number { return 7; } }\n",
                ),
                ("src/models/index.ts", index_src),
                (
                    "src/secret.test.ts",
                    "import { SecretHelper } from './models/secret';\ntest('bypasses', () => { expect(new SecretHelper().run()).toBe(7); });\n",
                ),
            ],
            &tc_cfg,
        )?;
        assert_eq!(
            diags_for(&diags, "test-bypasses-public-surface").len(),
            1,
            "{label}: {diags:?}"
        );
    }

    // `src/models.ts` has ModulePath `src::models`, making `src::models::secret` its child.
    let pure_reexport = analyze_project(
        &[
            ("src/models/public.ts", "export class PublicOrder {}\n"),
            (
                "src/models/secret.ts",
                "export class SecretHelper { run(): number { return 7; } }\n",
            ),
            (
                "src/models.ts",
                "export { PublicOrder } from './models/public';\n",
            ),
            (
                "src/secret.test.ts",
                "import { SecretHelper } from './models/secret';\ntest('direct import', () => { expect(new SecretHelper().run()).toBe(7); });\n",
            ),
        ],
        &tc_cfg,
    )?;
    assert_no_rule(&pure_reexport, "test-bypasses-public-surface");
    Ok(())
}

#[test]
fn barrel_mocking_propagates_external_boundary_through_barrels_and_empty_facades()
-> Result<(), Box<dyn Error>> {
    let tc_cfg = test_coupling_all_config();

    let mocking_diags = analyze_project(
        &[
            (
                "src/gateway/http.ts",
                "export async function fetchUser(id: string): Promise<unknown> { return fetch(id); }\n",
            ),
            (
                "src/gateway/index.ts",
                "export function version(): number { return 1; }\nexport { fetchUser } from './http';\n",
            ),
            (
                "src/gateway/bound_dir/index.ts",
                "import { fetchUser } from '../http';\nexport function version(): number { return 1; }\nexport { fetchUser };\n",
            ),
            (
                "src/gateway/all.ts",
                "export { fetchUser } from './http';\n",
            ),
            (
                "src/gateway/const_facade.ts",
                "export const VERSION = 1;\nexport { fetchUser } from './http';\n",
            ),
            (
                "src/gateway/helper.ts",
                "export function localCalc(): number { return 1; }\nexport { fetchUser } from './http';\n",
            ),
            (
                "src/service.test.ts",
                r#"
import { vi, test, expect } from "vitest";
vi.mock("./gateway", () => ({ fetchUser: vi.fn() }));
vi.mock("./gateway/bound_dir", () => ({ fetchUser: vi.fn() }));
vi.mock("./gateway/all", () => ({ fetchUser: vi.fn() }));
vi.mock("./gateway/const_facade", () => ({ fetchUser: vi.fn() }));
vi.mock("./gateway/helper", () => ({ localCalc: vi.fn() }));
test("mocks", () => { expect(1).toBe(1); });
"#,
            ),
        ],
        &tc_cfg,
    )?;
    let mock_hits = diags_for(&mocking_diags, "test-mocks-internal-collaborator");
    assert_eq!(
        mock_hits.len(),
        1,
        "only helper.ts should trigger: {mocking_diags:?}"
    );
    assert!(
        mock_hits[0].message().contains("./gateway/helper"),
        "{:?}",
        mock_hits[0]
    );
    Ok(())
}
