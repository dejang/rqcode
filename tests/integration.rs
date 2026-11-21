use rqcode::config::Config;
use rqcode::diagnostic::Diagnostic;
use rqcode::rules::ast_grep::AstGrepRuleError;
use rqcode::{AnalyzeError, Analyzer};
use std::error::Error;
use std::path::Path;

#[test]
fn loads_local_ast_grep_rules_dir_from_toml_on_top_of_embedded_rules() -> Result<(), Box<dyn Error>>
{
    let dir = tempfile::tempdir()?;
    let src = dir.path().join("src");
    let rules_dir = dir.path().join(".rqcode").join("rules");
    std::fs::create_dir_all(&src)?;
    std::fs::create_dir_all(&rules_dir)?;

    std::fs::write(
        dir.path().join("rqcode.toml"),
        r#"
[ast_grep]
rules_dir = ".rqcode/rules"
disabled_rules = ["no-dbg-macro"]
"#,
    )?;

    std::fs::write(
        rules_dir.join("no-println.yml"),
        r"
id: no-println-macro
language: Rust
severity: error
message: avoid `println!` in library code
note: Use structured logging instead.
rule:
  pattern: println!($$$ARGS)
",
    )?;

    std::fs::write(
        rules_dir.join("override-todo.yml"),
        r"
id: no-todo-macro
language: Rust
severity: warning
message: team policy forbids `todo!`
rule:
  pattern: todo!($$$ARGS)
",
    )?;

    std::fs::write(
        src.join("lib.rs"),
        r#"
pub fn run() {
    dbg!(42);
    println!("hello");
    todo!("finish later");
}
"#,
    )?;

    let config = Config::load_from_dir(dir.path())?;
    let configured_analyzer = Analyzer::from_config(&config, dir.path())?;
    let reports = [
        configured_analyzer.check_path(dir.path(), &config)?,
        Analyzer::default().check_path(dir.path(), &config)?,
    ];

    let mut by_id: Vec<(&str, &str)> = Vec::new();
    for report in &reports {
        by_id.clear();
        for diag in report.diagnostics() {
            by_id.push((diag.rule_id(), diag.message()));
        }
        by_id.sort_unstable();

        assert_eq!(
            by_id,
            vec![
                ("no-println-macro", "avoid `println!` in library code"),
                ("no-todo-macro", "team policy forbids `todo!`"),
            ]
        );
    }
    Ok(())
}

#[test]
fn disables_all_embedded_ast_grep_rules_and_rejects_missing_rules_dir() -> Result<(), Box<dyn Error>>
{
    let dir = tempfile::tempdir()?;
    let src = dir.path().join("src");
    let custom_dir = dir.path().join("custom-rules");
    std::fs::create_dir_all(&src)?;
    std::fs::create_dir_all(&custom_dir)?;

    std::fs::write(
        src.join("main.rs"),
        "fn main() { todo!(); unimplemented!(); dbg!(1); eprintln!(\"oops\"); }\n",
    )?;

    std::fs::write(
        custom_dir.join("no-eprintln.yaml"),
        r"
id: no-eprintln
language: Rust
severity: warning
message: avoid eprintln!
rule:
  pattern: eprintln!($$$ARGS)
",
    )?;

    let toml_path = dir.path().join("rqcode.toml");
    std::fs::write(
        &toml_path,
        r#"
[ast_grep]
builtin_rules = false
rules_dir = "custom-rules"
"#,
    )?;

    let config = Config::load_file(&toml_path)?;
    let analyzer = Analyzer::from_config(&config, dir.path())?;
    let report = analyzer.check_path(dir.path(), &config)?;

    let ids: Vec<&str> = report
        .diagnostics()
        .iter()
        .map(Diagnostic::rule_id)
        .collect();
    assert_eq!(ids, vec!["no-eprintln"]);

    std::fs::write(&toml_path, "[ast_grep]\nrules_dir = \"does-not-exist\"\n")?;
    let bad_config = Config::load_file(&toml_path)?;
    let result = Analyzer::from_config(&bad_config, dir.path());
    assert!(matches!(
        result,
        Err(AnalyzeError::AstGrepRule(
            AstGrepRuleError::MissingRulesDir(_)
        ))
    ));
    Ok(())
}

#[test]
fn rqcode_passes_its_own_quality_gate() -> Result<(), Box<dyn Error>> {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let config = Config::load_from_dir(manifest_dir)?;
    let report = Analyzer::from_config(&config, manifest_dir)?.check_path(manifest_dir, &config)?;

    assert!(
        report.is_clean(),
        "rqcode failed its own quality gate:\n{}",
        report.render_text()
    );
    Ok(())
}
