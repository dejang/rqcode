pub mod ast_grep;
pub mod cohesion;
pub mod similarity;
pub mod test_coupling;

use crate::ast::codebase::Codebase;
use crate::config::Config;
use crate::diagnostic::Diagnostic;
use crate::lang::LanguageRegistry;
use crate::rules::ast_grep::{AstGrepRuleError, AstGrepYamlRule, RuleOrigin};
use rayon::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Pluggable static analysis rule operating on the language-agnostic `Codebase`.
pub trait Rule: Send + Sync {
    fn id(&self) -> &str;
    fn description(&self) -> &str;
    fn check(&self, codebase: &Codebase, config: &Config) -> Vec<Diagnostic>;
}

/// Ordered collection of active analysis rules (built-in rules, embedded ast-grep YAML rules,
/// and project-specific ast-grep YAML rules loaded on top).
pub struct RuleRegistry {
    rules: Vec<Box<dyn Rule>>,
    loaded_ast_grep_dirs: Vec<PathBuf>,
}

impl RuleRegistry {
    #[must_use]
    pub fn empty() -> Self {
        Self {
            rules: Vec::new(),
            loaded_ast_grep_dirs: Vec::new(),
        }
    }

    /// Build a registry containing the core rules and all embedded ast-grep YAML rules.
    ///
    /// # Panics
    /// Panics if any compile-time embedded ast-grep YAML rule fails to parse.
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut registry = Self::core_rules();
        match ast_grep::load_embedded_rules(&LanguageRegistry::with_defaults()) {
            Ok(embedded) => {
                registry.register_yaml_rules(embedded);
            }
            Err(err) => panic!("embedded ast-grep YAML rules failed to parse: {err}"),
        }
        registry
    }

    /// Build a registry configured from `rqcode.toml`, applying `builtin_rules`, `disabled_rules`,
    /// and loading any custom ast-grep YAML rules from `ast_grep.rules_dir` on top of embedded rules,
    /// resolving rule languages via `languages`.
    ///
    /// # Errors
    /// Returns [`AstGrepRuleError`] if embedded rules or rules under `ast_grep.rules_dir` fail to load or parse.
    pub fn from_config(
        config: &Config,
        base_dir: &Path,
        languages: &LanguageRegistry,
    ) -> Result<Self, AstGrepRuleError> {
        let mut registry = Self::core_rules();

        if config.ast_grep.enabled {
            if config.ast_grep.builtin_rules {
                for rule in ast_grep::load_embedded_rules(languages)? {
                    if !config.ast_grep.is_rule_disabled(rule.id()) {
                        registry.register(Box::new(rule));
                    }
                }
            }

            if let Some(rules_dir) = config.resolve_ast_grep_rules_dir(base_dir) {
                let custom_rules = ast_grep::load_rules_from_dir(&rules_dir, languages)?;
                registry.record_loaded_dir(&rules_dir);
                for rule in custom_rules {
                    if !config.ast_grep.is_rule_disabled(rule.id()) {
                        registry.register(Box::new(rule));
                    }
                }
            }
        }

        Ok(registry)
    }

    fn core_rules() -> Self {
        Self {
            rules: vec![
                Box::new(similarity::SimilarityRule),
                Box::new(cohesion::CohesionRule),
                Box::new(test_coupling::TestCouplingRule),
                Box::new(ast_grep::AstGrepRule),
            ],
            loaded_ast_grep_dirs: Vec::new(),
        }
    }

    /// Register a rule, replacing any existing rule with the same ID in place.
    pub fn register(&mut self, rule: Box<dyn Rule>) {
        if let Some(existing) = self.rules.iter_mut().find(|r| r.id() == rule.id()) {
            *existing = rule;
        } else {
            self.rules.push(rule);
        }
    }

    fn register_yaml_rules(&mut self, rules: Vec<AstGrepYamlRule>) -> usize {
        let count = rules.len();
        for rule in rules {
            self.register(Box::new(rule));
        }
        count
    }

    /// Parse ast-grep YAML rule(s) and register them on top of existing rules.
    ///
    /// # Errors
    /// Returns [`AstGrepRuleError`] if the YAML or any rule file glob is invalid.
    pub fn load_ast_grep_yaml(
        &mut self,
        yaml: &str,
        languages: &LanguageRegistry,
    ) -> Result<usize, AstGrepRuleError> {
        let loaded = RuleOrigin::Inline.load_rules(yaml, languages)?;
        Ok(self.register_yaml_rules(loaded))
    }

    /// Load all ast-grep YAML rule files from `dir` and register them on top of existing rules.
    ///
    /// # Errors
    /// Returns [`AstGrepRuleError`] if `dir` does not exist, is not a directory, or contains invalid YAML rules.
    pub fn load_ast_grep_dir(
        &mut self,
        dir: &Path,
        languages: &LanguageRegistry,
    ) -> Result<usize, AstGrepRuleError> {
        let loaded = ast_grep::load_rules_from_dir(dir, languages)?;
        self.record_loaded_dir(dir);
        Ok(self.register_yaml_rules(loaded))
    }

    #[must_use]
    pub fn has_loaded_dir(&self, dir: &Path) -> bool {
        let normalized = normalize_dir_path(dir);
        self.loaded_ast_grep_dirs.iter().any(|d| d == &normalized)
    }

    #[must_use]
    pub fn rules(&self) -> &[Box<dyn Rule>] {
        &self.rules
    }

    #[must_use]
    pub fn run(&self, codebase: &Codebase, config: &Config) -> Vec<Diagnostic> {
        self.rules
            .par_iter()
            .flat_map(|rule| rule.check(codebase, config))
            .collect()
    }

    #[must_use]
    pub fn run_with_overlay(
        &self,
        codebase: &Codebase,
        config: &Config,
        overlay: &[AstGrepYamlRule],
    ) -> Vec<Diagnostic> {
        let overridden: HashSet<&str> = overlay.iter().map(Rule::id).collect();
        let mut diagnostics: Vec<Diagnostic> = self
            .rules
            .par_iter()
            .filter(|rule| !overridden.contains(rule.id()))
            .flat_map(|rule| rule.check(codebase, config))
            .collect();
        let overlay_diags: Vec<Diagnostic> = overlay
            .par_iter()
            .flat_map(|rule| rule.check(codebase, config))
            .collect();
        diagnostics.extend(overlay_diags);
        diagnostics
    }

    fn record_loaded_dir(&mut self, dir: &Path) {
        let normalized = normalize_dir_path(dir);
        if !self.loaded_ast_grep_dirs.contains(&normalized) {
            self.loaded_ast_grep_dirs.push(normalized);
        }
    }
}

fn normalize_dir_path(dir: &Path) -> PathBuf {
    std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_registry_includes_core_and_embedded_ast_grep_rules() {
        let registry = RuleRegistry::with_defaults();
        let ids: Vec<&str> = registry.rules().iter().map(|r| r.id()).collect();
        assert!(ids.contains(&"similarity"));
        assert!(ids.contains(&"cohesion"));
        assert!(ids.contains(&"test-coupling"));
        assert!(ids.contains(&"ast-grep"));
        assert!(ids.contains(&"no-todo-macro"));
        assert!(ids.contains(&"no-unimplemented-macro"));
        assert!(ids.contains(&"no-dbg-macro"));
    }

    #[test]
    fn from_config_layers_custom_dir_and_respects_disabling() {
        let dir = tempfile::tempdir().expect("tempdir");
        let rules_dir = dir.path().join("my-rules");
        std::fs::create_dir_all(&rules_dir).unwrap();
        std::fs::write(
            rules_dir.join("custom.yml"),
            r"
id: no-eprintln
language: Rust
severity: warning
message: avoid eprintln!
rule:
  pattern: eprintln!($$$ARGS)
---
id: no-todo-macro
language: Rust
severity: warning
message: custom todo override
rule:
  pattern: todo!($$$ARGS)
",
        )
        .unwrap();

        let mut cfg = Config::default();
        cfg.ast_grep.rules_dir = Some(PathBuf::from("my-rules"));
        cfg.ast_grep.disabled_rules = vec!["no-dbg-macro".to_string()];

        let languages = LanguageRegistry::with_defaults();
        let registry =
            RuleRegistry::from_config(&cfg, dir.path(), &languages).expect("build registry");
        assert!(registry.has_loaded_dir(&rules_dir));

        let ids: Vec<&str> = registry.rules().iter().map(|r| r.id()).collect();
        assert!(ids.contains(&"no-eprintln"));
        assert!(ids.contains(&"no-unimplemented-macro"));
        assert!(!ids.contains(&"no-dbg-macro"));

        let todo_rule = registry
            .rules()
            .iter()
            .find(|r| r.id() == "no-todo-macro")
            .expect("overridden no-todo-macro");
        assert_eq!(todo_rule.description(), "custom todo override");
    }

    #[test]
    fn from_config_omits_embedded_rules_when_builtin_rules_is_false() {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut cfg = Config::default();
        cfg.ast_grep.builtin_rules = false;

        let languages = LanguageRegistry::with_defaults();
        let registry =
            RuleRegistry::from_config(&cfg, dir.path(), &languages).expect("build registry");
        let ids: Vec<&str> = registry.rules().iter().map(|r| r.id()).collect();
        assert!(!ids.contains(&"no-todo-macro"));
        assert!(!ids.contains(&"no-dbg-macro"));
        assert!(ids.contains(&"similarity"));
        assert!(ids.contains(&"cohesion"));
        assert!(ids.contains(&"test-coupling"));
    }
}
