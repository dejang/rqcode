mod react_support;

use react_support::{check_project, rule_ids, write_project};
use rqcode::ast::item::TypeRef;
use rqcode::ast::{FileAst, SyntaxTree};
use rqcode::lang::LanguageRegistry;
use rqcode::source::SourceTree;
use std::error::Error;
use std::path::Path;

fn extract_file_ast(root: &Path, rel_target: &str) -> Result<FileAst, Box<dyn Error>> {
    let registry = LanguageRegistry::with_defaults();
    let tree = SourceTree::discover(root, &registry, &[])?;
    let file = tree
        .files()
        .iter()
        .find(|f| f.relative_path().to_string_lossy().replace('\\', "/") == rel_target)
        .ok_or_else(|| format!("{rel_target} not discovered"))?;
    let frontend = registry
        .frontend_for_path(file.relative_path())
        .ok_or_else(|| format!("no frontend for {rel_target}"))?;
    let syntax = SyntaxTree::parse(
        file.path(),
        file.relative_path(),
        file.language().clone(),
        file.content(),
    );
    Ok(frontend.extract(&syntax, root))
}

#[test]
fn discovers_typescript_and_tsx_extensions_including_uppercase() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    write_project(
        dir.path(),
        &[
            (
                "rqcode.toml",
                "[similarity]\nenabled = false\n[test_coupling]\nenabled = false\n[ast_grep]\nenabled = false\n",
            ),
            ("src/util.ts", "export function a(): number { return 1; }\n"),
            (
                "src/App.tsx",
                "import React from \"react\";\nexport function App() { return <main>ok</main>; }\n",
            ),
            ("src/mod.mts", "export const ver = 1;\n"),
            ("src/cjs.cts", "export const tag = \"c\";\n"),
            (
                "src/Banner.TSX",
                "import React from \"react\";\nfunction PromoTag() { return <span>sale</span>; }\nexport function Banner() { return <aside><PromoTag /></aside>; }\n",
            ),
        ],
    )?;

    let registry = LanguageRegistry::with_defaults();
    let tree = SourceTree::discover(dir.path(), &registry, &[])?;
    let mut rels: Vec<String> = tree
        .files()
        .iter()
        .map(|f| f.relative_path().to_string_lossy().replace('\\', "/"))
        .collect();
    rels.sort();

    assert_eq!(
        rels,
        vec![
            "src/App.tsx",
            "src/Banner.TSX",
            "src/cjs.cts",
            "src/mod.mts",
            "src/util.ts",
        ]
    );
    let banner_ast = extract_file_ast(dir.path(), "src/Banner.TSX")?;
    let banner_fn = banner_ast
        .functions
        .iter()
        .find(|f| f.name == "Banner")
        .ok_or("Banner function not extracted from uppercase .TSX")?;
    assert!(
        banner_fn.calls.iter().any(|c| c.callee == "PromoTag"),
        "uppercase .TSX must parse with TSX grammar and record JSX call `PromoTag`: {:?}",
        banner_fn.calls
    );
    let report = check_project(dir.path())?;
    assert!(
        report.is_clean(),
        "clean multi-extension TS/TSX project must check cleanly:\n{}",
        report.render_text()
    );
    Ok(())
}

#[test]
fn intrinsic_jsx_tags_do_not_hide_test_only_apis_or_fields() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    write_project(
        dir.path(),
        &[
            (
                "rqcode.toml",
                "[cohesion]\nenabled = false\n[similarity]\nenabled = false\n[ast_grep]\nenabled = false\n",
            ),
            (
                "src/main.tsx",
                "import React from \"react\";\nimport { createRoot } from \"react-dom/client\";\nimport { App } from \"./App\";\nconst el = document.getElementById(\"root\");\nif (el) { createRoot(el).render(<main><header><title>App</title><label>x</label><App /></header></main>); }\n",
            ),
            (
                "src/models/item.ts",
                r"export class Item {
  public title: string;
  public label: string;
  constructor(title: string, label: string) { this.title = title; this.label = label; }
  public describe(): string { return this.title; }
}
export function summary(item: Item): string { return item.describe(); }
",
            ),
            (
                "src/App.tsx",
                r#"import React from "react";
import { Item } from "./models/item";
export function App() {
  const item = new Item("book", "internal-only");
  return (
    <div><main><header><summary>{item.describe()}</summary>
      <form><label>Name</label><input type="text" /><my-widget data-name="demo" /></form>
    </header></main></div>
  );
}
"#,
            ),
            (
                "src/App.test.tsx",
                r#"import { describe, expect, it } from "vitest";
import { Item, summary } from "./models/item";
describe("Item", () => {
  it("calls summary, describe, and reads label", () => {
    const item = new Item("book", "debug-label");
    expect(item.describe()).toBe("book");
    expect(summary(item)).toBe("book");
    expect(item.label).toBe("debug-label");
  });
});
"#,
            ),
        ],
    )?;

    let report = check_project(dir.path())?;
    assert_eq!(
        rule_ids(&report),
        vec!["test-only-production-api", "test-only-production-field"],
        "expected exact test-only diagnostics (describe must stay reachable via App):\n{}",
        report.render_text()
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|d| d.rule_id() == "test-only-production-api" && d.message().contains("summary")),
        "expected test-only-production-api for `summary`:\n{}",
        report.render_text()
    );
    assert!(
        report
            .diagnostics()
            .iter()
            .any(|d| d.rule_id() == "test-only-production-field" && d.message().contains("label")),
        "expected test-only-production-field for `label`:\n{}",
        report.render_text()
    );
    Ok(())
}

#[test]
fn namespaced_and_compound_jsx_tags_are_calls_without_fake_constructions()
-> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    write_project(
        dir.path(),
        &[
            (
                "rqcode.toml",
                "[similarity]\nenabled = false\n[ast_grep]\nenabled = false\n",
            ),
            (
                "src/main.tsx",
                "import React from \"react\";\nimport { createRoot } from \"react-dom/client\";\nimport { Dashboard } from \"./Dashboard\";\nconst el = document.getElementById(\"root\");\nif (el) { createRoot(el).render(<Dashboard />); }\n",
            ),
            (
                "src/ui/cards.tsx",
                "import React from \"react\";\nexport function OrderCard({ name }: { name: string }) { return <article>{name}</article>; }\n",
            ),
            (
                "src/models/dialog.ts",
                "export class Dialog { private title: string; constructor(t: string) { this.title = t; } public heading(): string { return this.title; } }\n",
            ),
            (
                "src/Dashboard.tsx",
                r#"import React from "react";
import * as UI from "./ui/cards";
declare const Card: { Header: React.FC<{ children?: React.ReactNode }> };
declare const Dialog: { Root: React.FC<{ children?: React.ReactNode }> };
declare const ThemeContext: { Provider: React.FC<{ value: string; children?: React.ReactNode }> };
declare const Forms: { Fields: { Input: React.FC<{ name: string }> } };
export function Dashboard() {
  return (
    <ThemeContext.Provider value="light">
      <Dialog.Root><Card.Header>
        <UI.OrderCard name="INV-1" />
        <Forms.Fields.Input name="code" />
      </Card.Header></Dialog.Root>
    </ThemeContext.Provider>
  );
}
"#,
            ),
            (
                "src/Dashboard.test.tsx",
                r#"import React from "react";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { Dashboard } from "./Dashboard";
import { OrderCard } from "./ui/cards";
describe("Dashboard", () => {
  it("renders dashboard and order card", () => {
    render(<div><Dashboard /><OrderCard name="INV-2" /></div>);
    expect(screen.getByText("INV-2")).toBeInTheDocument();
  });
});
"#,
            ),
        ],
    )?;

    let report = check_project(dir.path())?;
    assert!(
        report.is_clean(),
        "expected clean diagnostics for namespaced/compound JSX tags:\n{}",
        report.render_text()
    );
    let ast = extract_file_ast(dir.path(), "src/Dashboard.tsx")?;
    let dashboard_fn = ast
        .functions
        .iter()
        .find(|f| f.name == "Dashboard")
        .ok_or("Dashboard function not extracted")?;
    assert!(
        dashboard_fn.field_accesses.is_empty(),
        "compound JSX tags must not record FieldAccess entries: {:?}",
        dashboard_fn.field_accesses
    );
    for forbidden in ["Dialog", "Card", "ThemeContext", "UI", "Forms"] {
        assert!(
            !dashboard_fn
                .constructed_types
                .contains(&TypeRef::Named(forbidden.to_string())),
            "compound JSX tag must not push constructed type `{forbidden}`: {:?}",
            dashboard_fn.constructed_types
        );
    }
    let callees: Vec<&str> = dashboard_fn
        .calls
        .iter()
        .map(|c| c.callee.as_str())
        .collect();
    for expected in ["Provider", "Root", "Header", "OrderCard", "Input"] {
        assert!(
            callees.contains(&expected),
            "expected `{expected}` in {callees:?}"
        );
    }
    Ok(())
}

#[test]
fn destructured_props_and_use_state_detect_external_mutations() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    write_project(
        dir.path(),
        &[
            (
                "rqcode.toml",
                "[test_coupling]\nenabled = false\n[similarity]\nenabled = false\n[ast_grep]\nenabled = false\n",
            ),
            (
                "src/models/order.ts",
                "export class Order {\n  public total: number = 0;\n  public label: string = \"\";\n}\n",
            ),
            (
                "src/components/TypeAliasPanel.tsx",
                r#"import React from "react";
import { Order } from "../models/order";
type BaseProps = { title: string };
type AliasProps = BaseProps & { order: Order };
export function TypeAliasPanel({ order, ...rest }: React.PropsWithChildren<AliasProps>) {
  return <button type="button" title={rest.title} onClick={() => { order.total = 10; }}>Reset</button>;
}
"#,
            ),
            (
                "src/components/ForwardRefPanel.tsx",
                r#"import React from "react";
import { Order } from "../models/order";
interface RefProps { order: Order; count?: number; }
export const ForwardRefPanel = React.forwardRef<HTMLDivElement, RefProps>(
  ({ order: current = new Order(), count = 0 }, ref) => {
    const [draft] = React.useState<Order>(current);
    return <div ref={ref} data-count={count} onClick={() => { draft.total = 42; }}>{draft.label}</div>;
  },
);
"#,
            ),
            (
                "src/components/CallbackListPanel.tsx",
                r#"import React from "react";
import { Order } from "../models/order";
interface Row { order: Order; }
export function CallbackListPanel({ rows }: { rows: Row[] }) {
  rows.forEach(({ order }: Row) => { order.label = "synced"; });
  return <div>{rows.length}</div>;
}
"#,
            ),
            (
                "src/components/DefaultExprPanel.tsx",
                r#"import React from "react";
import { Order } from "../models/order";
export function DefaultExprPanel({
  order,
  value = (() => { order.total = 99; return 1; })(),
}: { order: Order; value?: number }) {
  return <span>{value}</span>;
}
"#,
            ),
        ],
    )?;

    let report = check_project(dir.path())?;
    let mut mut_files: Vec<String> = report
        .diagnostics()
        .iter()
        .filter(|d| d.rule_id() == "external-field-mutation")
        .map(|d| {
            d.span()
                .file()
                .strip_prefix(dir.path())
                .unwrap_or(d.span().file())
                .to_string_lossy()
                .replace('\\', "/")
        })
        .collect();
    mut_files.sort();
    mut_files.dedup();

    for expected_file in [
        "src/components/TypeAliasPanel.tsx",
        "src/components/ForwardRefPanel.tsx",
        "src/components/CallbackListPanel.tsx",
        "src/components/DefaultExprPanel.tsx",
    ] {
        assert!(
            mut_files.contains(&String::from(expected_file)),
            "expected external-field-mutation in {expected_file}:\n{}",
            report.render_text()
        );
    }
    Ok(())
}

#[test]
fn destructured_props_preserve_positional_arity() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    write_project(
        dir.path(),
        &[
            (
                "rqcode.toml",
                "[test_coupling]\nenabled = false\n[similarity]\nenabled = false\n[ast_grep]\nenabled = false\n",
            ),
            (
                "src/models/order.ts",
                "export class Order {\n  public label: string = \"\";\n}\n",
            ),
            (
                "src/components/MultiParamArity.tsx",
                r#"import React from "react";
import { Order } from "../models/order";
interface Pair { first: string; second: string; }
function readPair({ first, second }: Pair, order: Order): string {
  return `${first}:${second}:${order.label}`;
}
export function ArityViewer({ pair, order }: { pair: Pair; order: Order }) {
  return <span>{readPair(pair, order)}</span>;
}
"#,
            ),
        ],
    )?;

    let report = check_project(dir.path())?;
    assert!(
        report.is_clean(),
        "read-only multi-param destructuring must stay clean:\n{}",
        report.render_text()
    );
    let ast = extract_file_ast(dir.path(), "src/components/MultiParamArity.tsx")?;
    let read_pair = ast
        .functions
        .iter()
        .find(|f| f.name == "readPair")
        .ok_or("readPair function not extracted")?;
    assert_eq!(
        read_pair.params.len(),
        2,
        "destructured first parameter must preserve positional arity: {:?}",
        read_pair.params
    );
    assert!(
        read_pair.params[0].name.is_empty(),
        "destructured parameter must use empty placeholder name: {:?}",
        read_pair.params[0]
    );
    assert_eq!(read_pair.params[1].name, "order");
    Ok(())
}
