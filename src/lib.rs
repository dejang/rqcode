pub mod ast;
pub mod config;
pub mod diagnostic;
pub mod lang;
pub mod report;
pub mod rules;
pub mod source;

use crate::ast::codebase::Codebase;
use crate::ast::{FileAst, SyntaxTree};
use crate::config::{Config, ConfigError};
use crate::lang::LanguageRegistry;
use crate::report::Report;
use crate::rules::RuleRegistry;
use crate::rules::ast_grep::{AstGrepRuleError, AstGrepYamlRule, load_rules_from_dir};
use crate::source::{SourceError, SourceTree};
use rayon::prelude::*;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AnalyzeError {
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Source(#[from] SourceError),
    #[error(transparent)]
    AstGrepRule(#[from] AstGrepRuleError),
}

/// Top-level orchestrator for the `parse -> ast -> rules -> report` pipeline.
pub struct Analyzer {
    languages: LanguageRegistry,
    rules: RuleRegistry,
}

impl Default for Analyzer {
    fn default() -> Self {
        Self::new(
            LanguageRegistry::with_defaults(),
            RuleRegistry::with_defaults(),
        )
    }
}

impl Analyzer {
    #[must_use]
    pub fn new(languages: LanguageRegistry, rules: RuleRegistry) -> Self {
        Self { languages, rules }
    }

    /// Build an `Analyzer` whose `RuleRegistry` is configured from `config` (applying embedded rule
    /// toggles, `disabled_rules`, and loading any custom ast-grep YAML rules from `rules_dir`).
    ///
    /// # Errors
    /// Returns [`AnalyzeError::AstGrepRule`] if embedded or configured ast-grep YAML rules fail to load or parse.
    pub fn from_config(config: &Config, base_dir: &Path) -> Result<Self, AnalyzeError> {
        let languages = LanguageRegistry::with_defaults();
        let rules = RuleRegistry::from_config(config, base_dir, &languages)?;
        Ok(Self::new(languages, rules))
    }

    #[must_use]
    pub fn languages(&self) -> &LanguageRegistry {
        &self.languages
    }

    #[must_use]
    pub fn rules(&self) -> &RuleRegistry {
        &self.rules
    }

    /// Discover files under `root`, parse them into the language-agnostic AST, and run all rules.
    ///
    /// # Errors
    /// Returns [`AnalyzeError::Source`] if file discovery fails or [`AnalyzeError::AstGrepRule`] if
    /// an overlay `ast_grep.rules_dir` cannot be loaded.
    pub fn check_path(&self, root: &Path, config: &Config) -> Result<Report, AnalyzeError> {
        let sources = SourceTree::discover(root, &self.languages, &config.exclude)?;
        let overlay = if config.ast_grep.enabled
            && let Some(rules_dir) = config.resolve_ast_grep_rules_dir(root)
            && !self.rules.has_loaded_dir(&rules_dir)
        {
            load_rules_from_dir(&rules_dir, &self.languages)?
        } else {
            Vec::new()
        };
        Ok(self.check_sources_with_overlay(&sources, config, &overlay))
    }

    /// Parse an already-loaded `SourceTree` and run all active rules.
    #[must_use]
    pub fn check_sources(&self, sources: &SourceTree, config: &Config) -> Report {
        self.check_sources_with_overlay(sources, config, &[])
    }

    fn check_sources_with_overlay(
        &self,
        sources: &SourceTree,
        config: &Config,
        overlay: &[AstGrepYamlRule],
    ) -> Report {
        let codebase = self.parse_codebase(sources);
        let raw_diagnostics = if overlay.is_empty() {
            self.rules.run(&codebase, config)
        } else {
            self.rules.run_with_overlay(&codebase, config, overlay)
        };
        Report::from_raw(&codebase, raw_diagnostics)
    }

    fn parse_codebase(&self, sources: &SourceTree) -> Codebase {
        let root = sources.root();
        let parsed: Vec<(SyntaxTree, FileAst)> = sources
            .files()
            .par_iter()
            .filter_map(|file| {
                let frontend = self.languages.frontend_for_id(file.language_id())?;
                let tree = SyntaxTree::parse(
                    file.path(),
                    file.relative_path(),
                    file.language().clone(),
                    file.content(),
                );
                let ast = frontend.extract(&tree, root);
                Some((tree, ast))
            })
            .collect();

        let (trees, files): (Vec<_>, Vec<_>) = parsed.into_iter().unzip();
        Codebase::build(trees, files)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::rust::rust_language_spec;
    use crate::source::SourceFile;

    #[test]
    fn analyzer_runs_full_pipeline_in_memory() {
        let sources = SourceTree::from_files(
            ".",
            vec![SourceFile::new(
                "src/lib.rs",
                "src/lib.rs",
                rust_language_spec(),
                "pub struct Counter { value: usize }\nimpl Counter { pub fn new(value: usize) -> Self { Self { value } } pub fn inc(&mut self) { self.value += 1; } }\n#[test] fn inc_works() { let mut c = Counter::new(0); c.inc(); }\n",
            )],
        );

        let report = Analyzer::default().check_sources(&sources, &Config::default());
        assert!(
            report.is_clean(),
            "unexpected report:\n{}",
            report.render_text()
        );
        assert_eq!(report.stats().files_analyzed, 1);
        assert_eq!(report.stats().types_analyzed, 1);
    }
}
