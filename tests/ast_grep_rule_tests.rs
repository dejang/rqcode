use ast_grep_config::from_str;
use rqcode::ast::SyntaxTree;
use rqcode::ast::codebase::Codebase;
use rqcode::config::Config;
use rqcode::lang::rust::{RustFrontend, rust_language_spec};
use rqcode::lang::{LanguageFrontend, LanguageRegistry};
use rqcode::rules::Rule;
use rqcode::rules::ast_grep::load_embedded_rules;
use serde::Deserialize;
use std::collections::HashMap;
use std::error::Error;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
struct AstGrepRuleTestSuite {
    id: String,
    #[serde(default)]
    valid: Vec<String>,
    #[serde(default)]
    invalid: Vec<String>,
}

fn check_snippet(rule: &dyn Rule, src: &str, out: &mut Vec<String>) {
    out.clear();
    let tree = SyntaxTree::parse("src/probe.rs", "src/probe.rs", rust_language_spec(), src);
    let ast = RustFrontend.extract(&tree, Path::new("."));
    let codebase = Codebase::build(vec![tree], vec![ast]);
    for d in rule.check(&codebase, &Config::default()) {
        out.push(format!("{} ({}): {}", d.rule_id(), d.span(), d.message()));
    }
}

#[test]
fn embedded_ast_grep_rules_pass_all_yaml_rule_tests() -> Result<(), Box<dyn Error>> {
    let registry = LanguageRegistry::with_defaults();
    let rules = load_embedded_rules(&registry)?;

    let mut rules_by_id = HashMap::new();
    for rule in &rules {
        let prev = rules_by_id.insert(rule.id(), rule);
        assert!(
            prev.is_none(),
            "duplicate embedded ast-grep rule id `{}`",
            rule.id()
        );
    }

    let test_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/rule_tests");
    let mut test_files: Vec<PathBuf> = std::fs::read_dir(&test_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with("-test.yml") || n.ends_with("-test.yaml"))
        })
        .collect();
    test_files.sort();
    assert!(
        !test_files.is_empty(),
        "expected YAML rule test files in {}",
        test_dir.display()
    );

    let mut failures = Vec::new();
    let mut diags = Vec::new();
    for path in &test_files {
        let content = std::fs::read_to_string(path)?;
        let suite: AstGrepRuleTestSuite = from_str(&content)?;
        let Some(&rule) = rules_by_id.get(suite.id.as_str()) else {
            failures.push(format!(
                "{}: no embedded ast-grep rule with id `{}`",
                path.display(),
                suite.id
            ));
            continue;
        };

        for (idx, snippet) in suite.valid.iter().enumerate() {
            check_snippet(rule, snippet, &mut diags);
            if !diags.is_empty() {
                failures.push(format!(
                    "{} [valid #{idx}] rule `{}` unexpectedly matched:\n--- snippet ---\n{snippet}\n--- diagnostics ---\n{}",
                    path.display(),
                    suite.id,
                    diags.join("\n")
                ));
            }
        }

        for (idx, snippet) in suite.invalid.iter().enumerate() {
            check_snippet(rule, snippet, &mut diags);
            if diags.is_empty() {
                failures.push(format!(
                    "{} [invalid #{idx}] rule `{}` failed to match:\n--- snippet ---\n{snippet}",
                    path.display(),
                    suite.id
                ));
            }
        }
    }

    assert!(
        failures.is_empty(),
        "ast-grep rule test failures:\n\n{}",
        failures.join("\n\n")
    );
    Ok(())
}
