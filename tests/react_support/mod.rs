use rqcode::Analyzer;
use rqcode::config::Config;
use rqcode::diagnostic::Diagnostic;
use rqcode::report::Report;
use std::error::Error;
use std::path::Path;

pub(crate) fn write_project(root: &Path, files: &[(&str, &str)]) -> Result<(), Box<dyn Error>> {
    for &(rel, content) in files {
        let full = root.join(rel);
        if let Some(parent) = full.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(full, content)?;
    }
    Ok(())
}

pub(crate) fn check_project(root: &Path) -> Result<Report, Box<dyn Error>> {
    let config = Config::load_from_dir(root)?;
    let analyzer = Analyzer::from_config(&config, root)?;
    Ok(analyzer.check_path(root, &config)?)
}

pub(crate) fn rule_ids(report: &Report) -> Vec<&str> {
    let mut ids: Vec<&str> = report
        .diagnostics()
        .iter()
        .map(Diagnostic::rule_id)
        .collect();
    ids.sort_unstable();
    ids
}
