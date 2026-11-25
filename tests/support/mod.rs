use rqcode::Analyzer;
use rqcode::config::Config;
use rqcode::diagnostic::Diagnostic;
use std::error::Error;

pub(crate) fn cohesion_only_config() -> Config {
    let mut cfg = Config::default();
    cfg.cohesion.enabled = true;
    cfg.similarity.enabled = false;
    cfg.test_coupling.enabled = false;
    cfg.ast_grep.enabled = false;
    cfg.ast_grep.builtin_rules = false;
    cfg
}

pub(crate) fn analyze_project(
    files: &[(&str, &str)],
    config: &Config,
) -> Result<Vec<Diagnostic>, Box<dyn Error>> {
    let dir = tempfile::tempdir()?;
    for (rel_path, content) in files {
        let full = dir.path().join(rel_path);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(full, content)?;
    }
    let report = Analyzer::from_config(config, dir.path())?.check_path(dir.path(), config)?;
    Ok(report.diagnostics().to_vec())
}

pub(crate) fn diags_for<'a>(diags: &'a [Diagnostic], rule_id: &str) -> Vec<&'a Diagnostic> {
    diags.iter().filter(|d| d.rule_id() == rule_id).collect()
}

pub(crate) fn assert_no_rule(diags: &[Diagnostic], rule_id: &str) {
    let hits = diags_for(diags, rule_id);
    assert!(
        hits.is_empty(),
        "expected 0 `{rule_id}` diagnostics, got: {hits:?}"
    );
}
