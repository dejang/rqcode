mod react_support;

use react_support::{check_project, rule_ids, write_project};
use std::error::Error;

#[test]
fn cra_side_effect_index_tsx_vs_library_index_tsx() -> Result<(), Box<dyn Error>> {
    let cra_dir = tempfile::tempdir()?;
    write_project(
        cra_dir.path(),
        &[
            (
                "rqcode.toml",
                "[cohesion]\nenabled = false\n[similarity]\nenabled = false\n[ast_grep]\nenabled = false\n",
            ),
            (
                "src/index.tsx",
                r#"import React from "react";
import { createRoot } from "react-dom/client";
import { App } from "./App";
const rootEl = document.getElementById("root");
if (rootEl) { createRoot(rootEl).render(<App />); }
"#,
            ),
            (
                "src/App.tsx",
                "import React from \"react\";\nexport function App() { return <main>ok</main>; }\n",
            ),
            (
                "src/UnusedPanel.tsx",
                "import React from \"react\";\nexport function UnusedPanel() { return <div>dead</div>; }\n",
            ),
            (
                "src/UnusedPanel.test.tsx",
                r#"import React from "react";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { UnusedPanel } from "./UnusedPanel";
describe("UnusedPanel", () => {
  it("renders unused panel", () => {
    render(<UnusedPanel />);
    expect(screen.getByText("dead")).toBeInTheDocument();
  });
});
"#,
            ),
        ],
    )?;

    let cra_report = check_project(cra_dir.path())?;
    assert!(
        rule_ids(&cra_report).contains(&"test-only-production-api"),
        "CRA-style side-effect src/index.tsx should enable binary-only test-only-production-api:\n{}",
        cra_report.render_text()
    );

    let lib_dir = tempfile::tempdir()?;
    write_project(
        lib_dir.path(),
        &[
            (
                "rqcode.toml",
                "[cohesion]\nenabled = false\n[similarity]\nenabled = false\n[ast_grep]\nenabled = false\n",
            ),
            (
                "src/index.tsx",
                "export { PublicPanel } from \"./PublicPanel\";\n",
            ),
            (
                "src/PublicPanel.tsx",
                "import React from \"react\";\nexport function PublicPanel() { return <div>pub</div>; }\n",
            ),
            (
                "src/PublicPanel.test.tsx",
                r#"import React from "react";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { PublicPanel } from "./index";
describe("PublicPanel", () => {
  it("renders exported library component", () => {
    render(<PublicPanel />);
    expect(screen.getByText("pub")).toBeInTheDocument();
  });
});
"#,
            ),
        ],
    )?;

    let lib_report = check_project(lib_dir.path())?;
    assert!(
        lib_report.is_clean(),
        "library src/index.tsx with exports must stay LibraryRoot:\n{}",
        lib_report.render_text()
    );
    Ok(())
}

#[test]
fn paired_jsx_suppressions_silence_real_diagnostics() -> Result<(), Box<dyn Error>> {
    let base_toml = "[similarity]\nenabled = false\n[ast_grep]\nbuiltin_rules = false\n\n[[ast_grep.rules]]\nid = \"no-danger-html\"\nlanguage = \"tsx\"\nseverity = \"error\"\nmessage = \"avoid dangerouslySetInnerHTML\"\npattern = \"<div dangerouslySetInnerHTML={$VAL} />\"\n";
    let order_src = "export class Order { public total: number = 0; }\n";

    let unsuppressed = tempfile::tempdir()?;
    write_project(
        unsuppressed.path(),
        &[
            ("rqcode.toml", base_toml),
            ("src/order.ts", order_src),
            (
                "src/Panel.tsx",
                r#"import React from "react";
import { Order } from "./order";
export const Panel = React.memo(({ order, html }: { order: Order; html: string }) => (
  <section>
    <div dangerouslySetInnerHTML={{ __html: html }} />
    {process.env.NODE_ENV === "test" && <span>debug</span>}
    <button type="button" onClick={() => { order.total = 0; }}>Clear</button>
  </section>
));
"#,
            ),
        ],
    )?;
    let baseline = check_project(unsuppressed.path())?;
    assert_eq!(
        rule_ids(&baseline),
        vec![
            "external-field-mutation",
            "no-danger-html",
            "production-test-hook",
        ],
        "baseline must fire external-field-mutation, no-danger-html, and production-test-hook:\n{}",
        baseline.render_text()
    );

    let line_suppressed = tempfile::tempdir()?;
    write_project(
        line_suppressed.path(),
        &[
            ("rqcode.toml", base_toml),
            ("src/order.ts", order_src),
            (
                "src/Panel.tsx",
                r#"import React from "react";
import { Order } from "./order";
// rqcode:allow(external-field-mutation)
export const Panel = React.memo(({ order, html }: { order: Order; html: string }) => (
  <section>
    {/* rqcode:allow(no-danger-html) */}
    <div dangerouslySetInnerHTML={{ __html: html }} />
    {/* rqcode:allow(production-test-hook) */}
    {process.env.NODE_ENV === "test" && <span>debug</span>}
    <button type="button" onClick={() => { order.total = 0; }}>Clear</button>
  </section>
));
"#,
            ),
        ],
    )?;
    let line_report = check_project(line_suppressed.path())?;
    assert!(
        line_report.is_clean(),
        "JSX and wrapper line suppressions must silence all three diagnostics:\n{}",
        line_report.render_text()
    );

    let file_suppressed = tempfile::tempdir()?;
    write_project(
        file_suppressed.path(),
        &[
            ("rqcode.toml", base_toml),
            ("src/order.ts", order_src),
            (
                "src/Panel.tsx",
                r#"import React from "react";
import { Order } from "./order";
export const Panel = React.memo(({ order, html }: { order: Order; html: string }) => (
  <section>
    {/* rqcode:allow-file(no-danger-html, production-test-hook, external-field-mutation) */}
    <div dangerouslySetInnerHTML={{ __html: html }} />
    {process.env.NODE_ENV === "test" && <span>debug</span>}
    <button type="button" onClick={() => { order.total = 0; }}>Clear</button>
  </section>
));
"#,
            ),
        ],
    )?;
    let file_report = check_project(file_suppressed.path())?;
    assert!(
        file_report.is_clean(),
        "JSX allow-file suppression must silence file diagnostics:\n{}",
        file_report.render_text()
    );
    Ok(())
}

#[test]
fn multiline_react_memo_wrapper_suppression_silences_duplicate() -> Result<(), Box<dyn Error>> {
    let sim_toml = "[cohesion]\nenabled = false\n[test_coupling]\nenabled = false\n[ast_grep]\nenabled = false\n[similarity]\nmin_tokens = 15\n";
    let card_a_body = r#"export const AlphaCard = React.memo(
  ({ title, amount, note }: { title: string; amount: number; note: string }) => {
    const label = amount > 0 ? `${title}: ${amount}` : `${title}: empty`;
    return (
      <article className="summary-card">
        <h3>{label}</h3>
        <p>{note}</p>
      </article>
    );
  },
);
"#;
    let card_a_unsuppressed = format!("import React from \"react\";\n\n{card_a_body}");
    let card_a_suppressed = format!(
        "import React from \"react\";\n\n// rqcode:allow(duplicate-function)\n{card_a_body}"
    );
    let card_b = r#"import React from "react";

export const BetaCard = React.memo(
  ({ heading, total, detail }: { heading: string; total: number; detail: string }) => {
    const caption = total > 0 ? `${heading}: ${total}` : `${heading}: empty`;
    return (
      <article className="summary-card">
        <h3>{caption}</h3>
        <p>{detail}</p>
      </article>
    );
  },
);
"#;

    let wrapper_base = tempfile::tempdir()?;
    write_project(
        wrapper_base.path(),
        &[
            ("rqcode.toml", sim_toml),
            ("src/AlphaCard.tsx", &card_a_unsuppressed),
            ("src/BetaCard.tsx", card_b),
        ],
    )?;
    let wrapper_base_report = check_project(wrapper_base.path())?;
    assert!(
        rule_ids(&wrapper_base_report).contains(&"duplicate-function"),
        "multiline React.memo duplicate pair must fire duplicate-function without suppression:\n{}",
        wrapper_base_report.render_text()
    );

    let wrapper_suppressed = tempfile::tempdir()?;
    write_project(
        wrapper_suppressed.path(),
        &[
            ("rqcode.toml", sim_toml),
            ("src/AlphaCard.tsx", &card_a_suppressed),
            ("src/BetaCard.tsx", card_b),
        ],
    )?;
    let wrapper_suppressed_report = check_project(wrapper_suppressed.path())?;
    assert!(
        wrapper_suppressed_report.is_clean(),
        "rqcode:allow(duplicate-function) above multiline React.memo primary wrapper must silence diagnostic:\n{}",
        wrapper_suppressed_report.render_text()
    );
    Ok(())
}

#[test]
fn curried_hoc_export_default_connect_tracks_live_and_test_only_components()
-> Result<(), Box<dyn Error>> {
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
                "import React from \"react\";\nimport { createRoot } from \"react-dom/client\";\nimport ConnectedBanner from \"./ConnectedBanner\";\nconst el = document.getElementById(\"root\");\nif (el) { createRoot(el).render(<ConnectedBanner title=\"ok\" />); }\n",
            ),
            (
                "src/hoc.tsx",
                "import React from \"react\";\nexport function connect<P>(mapState: () => Record<string, unknown>): (c: React.ComponentType<P>) => React.ComponentType<P> {\n  return (Comp) => (props: P) => <Comp {...props} {...mapState()} />;\n}\n",
            ),
            (
                "src/ConnectedBanner.tsx",
                "import React from \"react\";\nimport { connect } from \"./hoc\";\nexport default connect(() => ({}))(BannerView);\nfunction BannerView({ title }: { title: string }) { return <header>{title}</header>; }\n",
            ),
            (
                "src/DeadConnectedBanner.tsx",
                "import React from \"react\";\nimport { connect } from \"./hoc\";\nexport default connect(() => ({}))(DeadBannerView);\nfunction DeadBannerView({ title }: { title: string }) { return <aside>{title}</aside>; }\n",
            ),
            (
                "src/ConnectedBanner.test.tsx",
                r#"import React from "react";
import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import ConnectedBanner from "./ConnectedBanner";
import DeadConnectedBanner from "./DeadConnectedBanner";
describe("ConnectedBanner", () => {
  it("renders live and dead connected banners", () => {
    render(<div><ConnectedBanner title="hello" /><DeadConnectedBanner title="dead" /></div>);
    expect(screen.getByText("hello")).toBeInTheDocument();
  });
});
"#,
            ),
        ],
    )?;

    let report = check_project(dir.path())?;
    assert_eq!(
        rule_ids(&report),
        vec!["test-only-production-api"],
        "curried export default connect(...) should flag only DeadBannerView:\n{}",
        report.render_text()
    );
    assert!(
        report.diagnostics()[0].message().contains("DeadBannerView"),
        "expected DeadBannerView in diagnostic:\n{}",
        report.render_text()
    );
    Ok(())
}

#[test]
fn ast_grep_tsx_rules_respect_skip_tests_and_isolate_from_ts_rules() -> Result<(), Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    write_project(
        dir.path(),
        &[
            (
                "rqcode.toml",
                "[cohesion]\nenabled = false\n[test_coupling]\nenabled = false\n[similarity]\nenabled = false\n[ast_grep]\nbuiltin_rules = false\nrules_dir = \".rqcode/rules\"\n\n[[ast_grep.rules]]\nid = \"inline-no-danger\"\nlanguage = \"tsx\"\nseverity = \"warning\"\nmessage = \"inline rule forbids dangerouslySetInnerHTML\"\npattern = \"<div dangerouslySetInnerHTML={$VAL} />\"\n",
            ),
            (
                ".rqcode/rules/tsx-no-raw-img.yml",
                "id: tsx-no-raw-img\nlanguage: Tsx\nseverity: error\nmessage: use Image component instead of raw img\nmetadata:\n  skip_tests: true\nrule:\n  pattern: <img $$$ATTRS />\n",
            ),
            (
                ".rqcode/rules/ts-no-console.yml",
                "id: ts-no-console\nlanguage: TypeScript\nseverity: warning\nmessage: avoid console.log in ts modules\nrule:\n  pattern: console.log($$$ARGS)\n",
            ),
            (
                "src/Banner.tsx",
                "import React from \"react\";\nexport function Banner({ src, html }: { src: string; html: string }) {\n  console.log(src);\n  return <section><div dangerouslySetInnerHTML={{ __html: html }} /><img src={src} alt=\"hero\" /></section>;\n}\n",
            ),
            (
                "src/Banner.test.tsx",
                "import React from \"react\";\nimport { describe, it } from \"vitest\";\ndescribe(\"Banner\", () => {\n  it(\"renders raw img in test fixture\", () => { void (<img src=\"/a.png\" alt=\"test\" />); });\n});\n",
            ),
            (
                "src/components/__tests__/BannerPreview.tsx",
                "import React from \"react\";\nexport function preview() { return <img src=\"/preview.png\" alt=\"preview\" />; }\n",
            ),
            (
                "src/logger.ts",
                "export function logInfo(msg: string): void { console.log(msg); }\n",
            ),
            (
                "src/esmLogger.mts",
                "export function logEsm(msg: string): void { console.log(msg); }\n",
            ),
            (
                "src/cjsLogger.cts",
                "export function logCjs(msg: string): void { console.log(msg); }\n",
            ),
        ],
    )?;

    let report = check_project(dir.path())?;
    let mut hits: Vec<(&str, String)> = report
        .diagnostics()
        .iter()
        .map(|d| {
            let file = d.span().file();
            let rel = file
                .strip_prefix(dir.path())
                .unwrap_or(file)
                .to_string_lossy()
                .replace('\\', "/");
            (d.rule_id(), rel)
        })
        .collect();
    hits.sort();

    assert_eq!(
        hits,
        vec![
            ("inline-no-danger", String::from("src/Banner.tsx")),
            ("ts-no-console", String::from("src/cjsLogger.cts")),
            ("ts-no-console", String::from("src/esmLogger.mts")),
            ("ts-no-console", String::from("src/logger.ts")),
            ("tsx-no-raw-img", String::from("src/Banner.tsx")),
        ],
        "unexpected ast-grep hits:\n{}",
        report.render_text()
    );
    Ok(())
}
