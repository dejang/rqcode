mod react_support;

use react_support::{check_project, rule_ids, write_project};
use std::error::Error;

struct RegressionCase<'a> {
    label: &'a str,
    files: &'a [(&'a str, &'a str)],
    expected_rules: &'a [&'a str],
    expected_file_suffix: Option<&'a str>,
    expected_message_substr: Option<&'a str>,
}

#[test]
fn point5_cross_language_import_resolution_regressions() -> Result<(), Box<dyn Error>> {
    let tc_only =
        "[cohesion]\nenabled = false\n[similarity]\nenabled = false\n[ast_grep]\nenabled = false\n";
    let cases = [
        RegressionCase {
            label: "Rust inline #[cfg(test)] import resolves test call without shadowing prod",
            files: &[
                ("rqcode.toml", tc_only),
                (
                    "src/main.rs",
                    "mod other;\nmod worker;\nfn main() { let _ = worker::run(); }\n",
                ),
                ("src/other.rs", "pub fn helper() -> usize { 2 }\n"),
                (
                    "src/worker.rs",
                    "fn helper() -> usize { 1 }\npub fn run() -> usize { helper() }\n#[cfg(test)]\nmod tests {\n    use crate::other::helper;\n    #[test]\n    fn checks_other_helper() { assert_eq!(helper(), 2); assert_eq!(super::run(), 1); }\n}\n",
                ),
            ],
            expected_rules: &["test-only-production-api"],
            expected_file_suffix: Some("src/other.rs"),
            expected_message_substr: Some("helper"),
        },
        RegressionCase {
            label: "Rust integration test resolves aliased module re-export without shadowing",
            files: &[
                ("rqcode.toml", tc_only),
                (
                    "src/lib.rs",
                    "mod inner;\nmod shadow;\npub use inner as api;\n",
                ),
                ("src/inner.rs", "pub fn run() -> usize { 42 }\n"),
                (
                    "src/shadow.rs",
                    "fn run() -> usize { 0 }\npub fn keep() -> usize { run() }\n",
                ),
                (
                    "tests/integration.rs",
                    "use my_crate::api::run;\n#[test]\nfn runs_reexported_api() { assert_eq!(run(), 42); }\n",
                ),
            ],
            expected_rules: &[],
            expected_file_suffix: None,
            expected_message_substr: None,
        },
        RegressionCase {
            label: "Swift multi-file free-function resolution flags only debugOnlyScore",
            files: &[
                ("rqcode.toml", tc_only),
                (
                    "Sources/App/main.swift",
                    "let value = computeScore(10)\nprint(value)\n",
                ),
                (
                    "Sources/App/Scorer.swift",
                    "public func computeScore(_ n: Int) -> Int { return n * 2 }\npublic func debugOnlyScore(_ n: Int) -> Int { return n * 3 }\n",
                ),
                (
                    "Tests/AppTests/ScorerTests.swift",
                    "import XCTest\n@testable import App\nfinal class ScorerTests: XCTestCase {\n    func testScores() { XCTAssertEqual(computeScore(5), 10); XCTAssertEqual(debugOnlyScore(5), 15) }\n}\n",
                ),
            ],
            expected_rules: &["test-only-production-api"],
            expected_file_suffix: Some("Sources/App/Scorer.swift"),
            expected_message_substr: Some("debugOnlyScore"),
        },
    ];

    for case in &cases {
        let dir = tempfile::tempdir()?;
        write_project(dir.path(), case.files)?;
        let report = check_project(dir.path())?;
        assert_eq!(
            rule_ids(&report),
            case.expected_rules,
            "{}:\n{}",
            case.label,
            report.render_text()
        );
        if let Some(suffix) = case.expected_file_suffix {
            let file = report.diagnostics()[0]
                .span()
                .file()
                .to_string_lossy()
                .replace('\\', "/");
            assert!(
                file.ends_with(suffix),
                "{}: expected file `{suffix}`, got `{file}`",
                case.label
            );
        }
        if let Some(needle) = case.expected_message_substr {
            assert!(
                report.diagnostics()[0].message().contains(needle),
                "{}:\n{}",
                case.label,
                report.render_text()
            );
        }
    }
    Ok(())
}
