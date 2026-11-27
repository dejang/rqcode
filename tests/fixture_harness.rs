use rqcode::Analyzer;
use rqcode::config::Config;
use rqcode::diagnostic::Diagnostic;
use rqcode::report::Report;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct FixtureExpectation {
    rules: Option<Vec<String>>,
    exact: Option<bool>,
    diagnostics: Vec<ExpectedDiagnostic>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExpectedDiagnostic {
    rule: String,
    file: Option<String>,
    line: Option<usize>,
    message_contains: Option<String>,
    #[serde(default)]
    related_files: Vec<String>,
    #[serde(default)]
    related_contains: Vec<String>,
}

struct FixtureCase {
    name: String,
    dir: PathBuf,
    expectation: FixtureExpectation,
}

struct DiagnosticScratch {
    used_indices: HashSet<usize>,
    by_rule: HashMap<String, Vec<usize>>,
    actual_related_files: HashSet<String>,
    errors: Vec<String>,
}

impl FixtureExpectation {
    fn apply_rule_filter(&self, config: &mut Config) -> Result<(), String> {
        let Some(rules) = self.rules.as_ref() else {
            return Ok(());
        };
        config.cohesion.enabled = false;
        config.similarity.enabled = false;
        config.test_coupling.enabled = false;
        config.ast_grep.enabled = false;
        config.ast_grep.builtin_rules = false;

        for rule in rules {
            match rule.as_str() {
                "cohesion" => config.cohesion.enabled = true,
                "similarity" => config.similarity.enabled = true,
                "test-coupling" => config.test_coupling.enabled = true,
                "ast-grep" => {
                    config.ast_grep.enabled = true;
                    config.ast_grep.builtin_rules = true;
                }
                other => {
                    return Err(format!(
                        "unknown rule family `{other}` in expected.toml (expected cohesion, similarity, test-coupling, or ast-grep)"
                    ));
                }
            }
        }
        Ok(())
    }
}

impl ExpectedDiagnostic {
    fn matches(
        &self,
        diag: &Diagnostic,
        fixture_dir: &Path,
        related_files_buf: &mut HashSet<String>,
    ) -> bool {
        if diag.rule_id() != self.rule {
            return false;
        }
        if let Some(expected_file) = self.file.as_deref() {
            let actual_file = normalize_rel_path(diag.span().file(), fixture_dir);
            if actual_file != expected_file {
                return false;
            }
        }
        if let Some(expected_line) = self.line
            && diag.span().start_line() != expected_line
        {
            return false;
        }
        if let Some(substr) = self.message_contains.as_deref()
            && !diag.message().contains(substr)
        {
            return false;
        }
        if !self.related_files.is_empty() {
            related_files_buf.clear();
            for rel in diag.related() {
                related_files_buf.insert(normalize_rel_path(rel.span().file(), fixture_dir));
            }
            for expected_rel in &self.related_files {
                if !related_files_buf.contains(expected_rel) {
                    return false;
                }
            }
        }
        for needle in &self.related_contains {
            let mut found = false;
            for rel in diag.related() {
                if rel.label().contains(needle) {
                    found = true;
                    break;
                }
            }
            if !found {
                return false;
            }
        }
        true
    }
}

impl FixtureCase {
    fn discover_all(
        fixtures_root: &Path,
        skip_subdir: Option<&str>,
    ) -> Result<Vec<Self>, Box<dyn Error>> {
        let mut stack = vec![fixtures_root.to_path_buf()];
        let mut dirs = Vec::new();

        while let Some(dir) = stack.pop() {
            let manifest = dir.join("expected.toml");
            if manifest.is_file() {
                dirs.push(dir);
                continue;
            }
            for entry in std::fs::read_dir(&dir)? {
                let path = entry?.path();
                if path.is_dir() {
                    if let Some(skip) = skip_subdir
                        && dir == fixtures_root
                        && path.file_name().and_then(|n| n.to_str()) == Some(skip)
                    {
                        continue;
                    }
                    stack.push(path);
                }
            }
        }
        dirs.sort();

        let mut cases = Vec::with_capacity(dirs.len());
        for dir in dirs {
            let raw = std::fs::read_to_string(dir.join("expected.toml"))?;
            let expectation: FixtureExpectation = toml::from_str(&raw).map_err(|err| {
                format!(
                    "failed to parse {}: {err}",
                    dir.join("expected.toml").display()
                )
            })?;
            let name = normalize_rel_path(&dir, fixtures_root);
            cases.push(Self {
                name,
                dir,
                expectation,
            });
        }
        Ok(cases)
    }

    fn run(&self, scratch: &mut DiagnosticScratch) -> Result<(), Box<dyn Error>> {
        let mut config = Config::load_from_dir(&self.dir)?;
        if let Err(msg) = self.expectation.apply_rule_filter(&mut config) {
            scratch.errors.push(format!("[{}] {msg}", self.name));
            return Ok(());
        }
        let analyzer = Analyzer::from_config(&config, &self.dir)?;
        let report = analyzer.check_path(&self.dir, &config)?;
        self.verify_report(&report, scratch);
        Ok(())
    }

    fn verify_report(&self, report: &Report, scratch: &mut DiagnosticScratch) {
        let actual = report.diagnostics();
        scratch.used_indices.clear();
        scratch.by_rule.clear();
        for (idx, diag) in actual.iter().enumerate() {
            scratch
                .by_rule
                .entry(diag.rule_id().to_owned())
                .or_default()
                .push(idx);
        }

        let mut missing = String::new();
        for (exp_idx, expected) in self.expectation.diagnostics.iter().enumerate() {
            let mut matched_idx = None;
            if let Some(candidates) = scratch.by_rule.get(&expected.rule) {
                for &cand_idx in candidates {
                    if scratch.used_indices.contains(&cand_idx) {
                        continue;
                    }
                    if expected.matches(
                        &actual[cand_idx],
                        &self.dir,
                        &mut scratch.actual_related_files,
                    ) {
                        matched_idx = Some(cand_idx);
                        break;
                    }
                }
            }
            if let Some(idx) = matched_idx {
                scratch.used_indices.insert(idx);
            } else {
                let _ = writeln!(missing, "  - expected[{exp_idx}] not matched: {expected:?}");
            }
        }

        let exact = self.expectation.exact.unwrap_or(true);
        let mut unexpected = String::new();
        if exact {
            for (idx, diag) in actual.iter().enumerate() {
                if !scratch.used_indices.contains(&idx) {
                    let _ = writeln!(
                        unexpected,
                        "  - unexpected: {} ({}:{}): {}",
                        diag.rule_id(),
                        normalize_rel_path(diag.span().file(), &self.dir),
                        diag.span().start_line(),
                        diag.message()
                    );
                }
            }
        }

        if !missing.is_empty() || !unexpected.is_empty() {
            scratch.errors.push(format!(
                "=== fixture `{}` failed ===\n{missing}{unexpected}Actual diagnostics:\n{}",
                self.name,
                report.render_text()
            ));
        }
    }
}

fn normalize_rel_path(path: &Path, root: &Path) -> String {
    let rel = path.strip_prefix(root).unwrap_or(path);
    rel.to_string_lossy().replace('\\', "/")
}

fn run_fixture_cases(cases: &[FixtureCase], label: &str) -> Result<(), Box<dyn Error>> {
    assert!(!cases.is_empty(), "no fixture cases discovered for {label}");

    let mut scratch = DiagnosticScratch {
        used_indices: HashSet::new(),
        by_rule: HashMap::new(),
        actual_related_files: HashSet::new(),
        errors: Vec::new(),
    };
    for case in cases {
        case.run(&mut scratch)?;
    }

    assert!(
        scratch.errors.is_empty(),
        "{} of {} {label} fixture(s) failed:\n\n{}",
        scratch.errors.len(),
        cases.len(),
        scratch.errors.join("\n")
    );
    Ok(())
}

#[test]
fn multi_file_fixtures_match_expectations() -> Result<(), Box<dyn Error>> {
    let fixtures_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures");
    let cases = FixtureCase::discover_all(&fixtures_root, Some("react"))?;
    run_fixture_cases(&cases, "core")
}

#[test]
fn react_fixtures_match_expectations() -> Result<(), Box<dyn Error>> {
    let react_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("react");
    let cases = FixtureCase::discover_all(&react_root, None)?;
    run_fixture_cases(&cases, "react")
}
