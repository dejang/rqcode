mod loader;

pub use self::loader::{
    AstGrepRuleError, EMBEDDED_AST_GREP_RULES, RuleOrigin, load_embedded_rules,
    load_rules_from_dir, load_rules_from_file,
};

use crate::ast::codebase::{Codebase, FileId};
use crate::ast::{LanguageSpec, SyntaxTree};
use crate::config::Config;
use crate::diagnostic::{Diagnostic, Severity, Span};
use crate::rules::Rule;
use ast_grep_config::{
    CombinedScan, LabelStyle, RuleCollection, RuleConfig, Severity as AstGrepSeverity,
};
use ast_grep_core::NodeMatch;
use ast_grep_core::matcher::Matcher;
use ast_grep_core::tree_sitter::StrDoc;

struct ScanTarget<'a> {
    codebase: &'a Codebase,
    file_id: FileId,
    tree: &'a SyntaxTree,
    severity: Severity,
}

/// A compiled ast-grep YAML rule (embedded or loaded from a local YAML file) runnable via `Rule`.
pub struct AstGrepYamlRule {
    id: String,
    description: String,
    origin: RuleOrigin,
    skip_tests: bool,
    severity: Option<Severity>,
    collection: RuleCollection<LanguageSpec>,
}

impl AstGrepYamlRule {
    /// Compile a parsed `ast-grep` [`RuleConfig`] into a runnable [`AstGrepYamlRule`].
    ///
    /// # Errors
    /// Returns [`AstGrepRuleError::InvalidGlob`] if the rule's `files` or `ignores` globs are invalid.
    pub fn from_rule_config(
        config: RuleConfig<LanguageSpec>,
        origin: RuleOrigin,
    ) -> Result<Self, AstGrepRuleError> {
        let id = config.id.clone();
        let description = if config.message.trim().is_empty() {
            format!("Runs ast-grep YAML rule `{id}`")
        } else {
            config.message.clone()
        };
        let skip_tests = config
            .metadata
            .as_ref()
            .and_then(|meta| serde_json::to_value(meta).ok())
            .and_then(|val| val.get("skip_tests").and_then(serde_json::Value::as_bool))
            .unwrap_or(false);
        let severity = match config.severity {
            AstGrepSeverity::Off => None,
            AstGrepSeverity::Error => Some(Severity::Error),
            AstGrepSeverity::Warning | AstGrepSeverity::Info | AstGrepSeverity::Hint => {
                Some(Severity::Warning)
            }
        };
        let collection =
            RuleCollection::try_new(vec![config]).map_err(|err| AstGrepRuleError::InvalidGlob {
                origin: origin.clone(),
                rule_id: id.clone(),
                message: err.to_string(),
            })?;

        Ok(Self {
            id,
            description,
            origin,
            skip_tests,
            severity,
            collection,
        })
    }

    #[must_use]
    pub fn is_builtin(&self) -> bool {
        matches!(self.origin, RuleOrigin::Embedded(_))
    }

    #[must_use]
    pub fn origin(&self) -> &RuleOrigin {
        &self.origin
    }

    #[must_use]
    pub fn scan_tree(
        &self,
        codebase: &Codebase,
        file_id: FileId,
        tree: &SyntaxTree,
    ) -> Vec<Diagnostic> {
        let Some(severity) = self.severity else {
            return Vec::new();
        };

        let rules = self
            .collection
            .get_rule_from_lang(tree.relative_path(), tree.language().clone());
        let Some(&rule_cfg) = rules.first() else {
            return Vec::new();
        };

        let target = ScanTarget {
            codebase,
            file_id,
            tree,
            severity,
        };
        let mut diagnostics = Vec::new();
        if rule_cfg.matcher.potential_kinds().is_some() {
            let scan = CombinedScan::new(vec![rule_cfg]);
            let result = scan.scan(tree.ast_grep(), false);
            for (matched_rule, matches) in result.matches {
                for node_match in matches {
                    if let Some(diag) =
                        self.diagnostic_from_match(&target, matched_rule, &node_match)
                    {
                        diagnostics.push(diag);
                    }
                }
            }
        } else {
            for node_match in tree.root().find_all(&rule_cfg.matcher) {
                if let Some(diag) = self.diagnostic_from_match(&target, rule_cfg, &node_match) {
                    diagnostics.push(diag);
                }
            }
        }

        diagnostics
    }

    fn diagnostic_from_match(
        &self,
        target: &ScanTarget<'_>,
        rule_cfg: &RuleConfig<LanguageSpec>,
        node_match: &NodeMatch<'_, StrDoc<LanguageSpec>>,
    ) -> Option<Diagnostic> {
        let span = target.tree.node_span(node_match.get_node());
        if self.skip_tests
            && target
                .codebase
                .is_line_in_test_code(target.file_id, span.start_line())
        {
            return None;
        }

        let raw_message = rule_cfg.get_message(node_match);
        let message = if raw_message.trim().is_empty() {
            format!("matched ast-grep rule `{}`", rule_cfg.id)
        } else {
            raw_message
        };

        let mut diag = Diagnostic::new(rule_cfg.id.clone(), target.severity, span, message);
        if let Some(note) = &rule_cfg.note {
            let trimmed = note.trim();
            if !trimmed.is_empty() {
                diag = diag.with_suggestion(trimmed);
            }
        }

        for label in rule_cfg.get_labels(node_match) {
            if matches!(label.style, LabelStyle::Secondary) {
                let rel_span = target.tree.nodes_span(&label.start_node, &label.end_node);
                let rel_msg = label.message.unwrap_or("related match");
                diag = diag.with_related(rel_span, rel_msg);
            }
        }

        Some(diag)
    }
}

impl Rule for AstGrepYamlRule {
    fn id(&self) -> &str {
        &self.id
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn check(&self, codebase: &Codebase, config: &Config) -> Vec<Diagnostic> {
        let cfg = &config.ast_grep;
        if !cfg.enabled || cfg.is_rule_disabled(&self.id) {
            return Vec::new();
        }
        if self.is_builtin() && !cfg.builtin_rules {
            return Vec::new();
        }

        let mut diagnostics = Vec::new();
        for (file_idx, tree) in codebase.trees().iter().enumerate() {
            diagnostics.extend(self.scan_tree(codebase, FileId(file_idx), tree));
        }
        diagnostics
    }
}

/// Runs optional inline `[[ast_grep.rules]]` pattern checks configured directly in `rqcode.toml`.
pub struct AstGrepRule;

impl Rule for AstGrepRule {
    fn id(&self) -> &'static str {
        "ast-grep"
    }

    fn description(&self) -> &'static str {
        "Runs inline ast-grep pattern checks configured in rqcode.toml"
    }

    fn check(&self, codebase: &Codebase, config: &Config) -> Vec<Diagnostic> {
        let cfg = &config.ast_grep;
        if !cfg.enabled || cfg.rules.is_empty() {
            return Vec::new();
        }

        let mut diagnostics = Vec::new();

        for (file_idx, tree) in codebase.trees().iter().enumerate() {
            let file_id = FileId(file_idx);

            for pattern_rule in &cfg.rules {
                if cfg.is_rule_disabled(&pattern_rule.id) {
                    continue;
                }
                if !tree.language().matches_name(&pattern_rule.language) {
                    continue;
                }

                match tree.find_pattern(&pattern_rule.pattern) {
                    Ok(matches) => {
                        for m in matches {
                            if pattern_rule.skip_tests
                                && codebase.is_line_in_test_code(file_id, m.span().start_line())
                            {
                                continue;
                            }
                            let mut diag = Diagnostic::new(
                                pattern_rule.id.clone(),
                                pattern_rule.severity,
                                m.span().clone(),
                                pattern_rule.message.clone(),
                            );
                            if let Some(suggestion) = &pattern_rule.suggestion {
                                diag = diag.with_suggestion(suggestion.clone());
                            }
                            diagnostics.push(diag);
                        }
                    }
                    Err(err) => {
                        diagnostics.push(Diagnostic::new(
                            "invalid-ast-grep-pattern",
                            Severity::Error,
                            Span::new(tree.relative_path(), 1, 1, 1, 1),
                            err.to_string(),
                        ));
                    }
                }
            }
        }

        diagnostics
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::rust::{RustFrontend, rust_language_spec};
    use crate::lang::{LanguageFrontend, LanguageRegistry};
    use std::path::Path;

    fn parse_rust_codebase(rel_path: &str, src: &str) -> Codebase {
        let tree = SyntaxTree::parse(rel_path, rel_path, rust_language_spec(), src);
        let ast = RustFrontend.extract(&tree, Path::new("."));
        Codebase::build(vec![tree], vec![ast])
    }

    #[test]
    fn embedded_yaml_rules_parse_and_skip_test_code() {
        let registry = LanguageRegistry::with_defaults();
        let rules = load_embedded_rules(&registry).expect("embedded YAML rules should parse");
        let ids: Vec<&str> = rules.iter().map(Rule::id).collect();
        assert!(ids.contains(&"no-todo-macro"));
        assert!(ids.contains(&"no-unimplemented-macro"));
        assert!(ids.contains(&"no-dbg-macro"));
        assert!(ids.contains(&"no-redundant-wrapper"));
        assert!(ids.contains(&"no-private-forwarder"));
        assert!(ids.contains(&"impl-const"));

        let src = r#"
pub fn prod_fn() {
    todo!("implement me");
}

#[test]
fn test_fn() {
    todo!("allowed in tests when metadata.skip_tests = true");
}
"#;
        let codebase = parse_rust_codebase("src/lib.rs", src);
        let cfg = Config::default();

        let diags: Vec<Diagnostic> = rules
            .iter()
            .flat_map(|r| r.check(&codebase, &cfg))
            .collect();
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].rule_id(), "no-todo-macro");
        assert_eq!(diags[0].span().start_line(), 3);
        assert_eq!(
            diags[0].suggestion(),
            Some("Implement the missing logic or return a typed error.")
        );
    }

    #[test]
    fn embedded_rules_can_be_disabled_globally_or_individually() {
        let registry = LanguageRegistry::with_defaults();
        let rules = load_embedded_rules(&registry).expect("embedded YAML rules");
        let codebase = parse_rust_codebase("src/lib.rs", "pub fn f() { todo!(); dbg!(1); }\n");

        let mut disable_one = Config::default();
        disable_one.ast_grep.disabled_rules = vec!["no-dbg-macro".to_string()];
        let diags_one: Vec<String> = rules
            .iter()
            .flat_map(|r| r.check(&codebase, &disable_one))
            .map(|d| d.rule_id().to_string())
            .collect();
        assert_eq!(diags_one, vec!["no-todo-macro"]);

        let mut disable_builtins = Config::default();
        disable_builtins.ast_grep.builtin_rules = false;
        let builtins_disabled_diags: Vec<Diagnostic> = rules
            .iter()
            .flat_map(|r| r.check(&codebase, &disable_builtins))
            .collect();
        assert_eq!(builtins_disabled_diags, Vec::<Diagnostic>::new());
    }

    #[test]
    fn parses_multi_document_yaml_with_constraints_metavars_and_file_globs() {
        let registry = LanguageRegistry::with_defaults();
        let yaml = r#"
id: no-unwrap-in-src
language: Rust
severity: error
message: avoid `.unwrap()` on `$EXPR`
note: Propagate errors using `?` instead.
files:
  - "src/**/*.rs"
rule:
  pattern: $EXPR.unwrap()
---
id: disabled-by-severity
language: Rust
severity: off
message: should not fire
rule:
  pattern: $EXPR.expect($MSG)
"#;
        let rules = RuleOrigin::Inline
            .load_rules(yaml, &registry)
            .expect("valid multi-doc YAML");
        assert_eq!(rules.len(), 2);

        let src = "pub fn load(x: Option<i32>) -> i32 { x.unwrap() }\n";
        let in_src = parse_rust_codebase("src/app.rs", src);
        let in_other = parse_rust_codebase("examples/demo.rs", src);
        let cfg = Config::default();

        let src_diags: Vec<Diagnostic> =
            rules.iter().flat_map(|r| r.check(&in_src, &cfg)).collect();
        assert_eq!(src_diags.len(), 1);
        assert_eq!(src_diags[0].rule_id(), "no-unwrap-in-src");
        assert_eq!(src_diags[0].message(), "avoid `.unwrap()` on `x`");
        assert_eq!(
            src_diags[0].suggestion(),
            Some("Propagate errors using `?` instead.")
        );

        let other_diags: Vec<Diagnostic> = rules
            .iter()
            .flat_map(|r| r.check(&in_other, &cfg))
            .collect();
        assert_eq!(other_diags, Vec::<Diagnostic>::new());
    }

    #[test]
    fn loads_rules_recursively_from_directory_and_reports_errors() {
        let registry = LanguageRegistry::with_defaults();
        let dir = tempfile::tempdir().expect("tempdir");
        let nested = dir.path().join("rust");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(dir.path().join("README.md"), "# ignored").unwrap();
        std::fs::write(
            nested.join("no-println.yaml"),
            r"
id: no-println
language: Rust
severity: warning
message: avoid println!
rule:
  pattern: println!($$$ARGS)
",
        )
        .unwrap();

        let loaded = load_rules_from_dir(dir.path(), &registry).expect("should load directory");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].id(), "no-println");
        assert!(!loaded[0].is_builtin());

        let missing = load_rules_from_dir(&dir.path().join("does-not-exist"), &registry);
        assert!(matches!(missing, Err(AstGrepRuleError::MissingRulesDir(_))));

        let bad_yaml =
            RuleOrigin::Inline.load_rules("id: broken\nlanguage: Rust\nrule: [", &registry);
        assert!(matches!(bad_yaml, Err(AstGrepRuleError::ParseYaml { .. })));

        let bad_lang = RuleOrigin::Inline.load_rules(
            "id: unknown-lang\nlanguage: cobol\nseverity: error\nmessage: nope\nrule:\n  pattern: foo\n",
            &registry,
        );
        assert!(matches!(bad_lang, Err(AstGrepRuleError::ParseYaml { .. })));
    }

    #[test]
    fn const_rules_partition_module_local_and_inherent_impl_constants() {
        let registry = LanguageRegistry::with_defaults();
        let rules: Vec<AstGrepYamlRule> = load_embedded_rules(&registry)
            .expect("embedded YAML rules")
            .into_iter()
            .filter(|r| matches!(r.id(), "module-const" | "local-const" | "impl-const"))
            .collect();
        assert_eq!(rules.len(), 3);

        let src = r#"
const MOD_C: usize = 1;
pub trait Named { const NAME: &'static str; }
pub struct Probe;
impl Named for Probe { const NAME: &'static str = "probe"; }
impl Probe {
    const IMPL_C: usize = 2;
    fn method(&self) -> usize {
        const METHOD_C: usize = 3;
        METHOD_C
    }
}
fn scoped() -> usize {
    struct Local;
    impl Local { const FN_IMPL_C: usize = 4; }
    Local::FN_IMPL_C
}
"#;
        let codebase = parse_rust_codebase("src/consts.rs", src);
        let cfg = Config::default();
        let mut hits: Vec<(usize, String)> = rules
            .iter()
            .flat_map(|r| r.check(&codebase, &cfg))
            .map(|d| (d.span().start_line(), d.rule_id().to_string()))
            .collect();
        hits.sort();
        assert_eq!(
            hits,
            vec![
                (2, "module-const".to_string()),
                (7, "impl-const".to_string()),
                (9, "local-const".to_string()),
                (15, "local-const".to_string()),
            ]
        );
    }

    #[test]
    fn wrapper_rules_catch_redundant_wrappers_and_skip_facades() {
        let registry = LanguageRegistry::with_defaults();
        let rules: Vec<AstGrepYamlRule> = load_embedded_rules(&registry)
            .expect("embedded YAML rules")
            .into_iter()
            .filter(|r| matches!(r.id(), "no-redundant-wrapper" | "no-private-forwarder"))
            .collect();

        let src = r"
pub struct Loader { base: usize }
impl Loader {
    pub fn load_rules(&self, yaml: &str) -> usize { self.load_rules_with_registry(yaml, &Registry::default()) }
    pub fn load_rules_with_registry(&self, yaml: &str, _reg: &Registry) -> usize { self.base + yaml.len() }
    pub fn from_config(cfg: &str) -> Self { Self::from_config_with_registry(cfg, &Registry::default()) }
    pub fn from_config_with_registry(cfg: &str, _reg: &Registry) -> Self { Self { base: cfg.len() } }
    pub fn check_sources(&self, src: &str) -> usize { self.check_sources_with_overlay(src, &[]) }
    fn check_sources_with_overlay(&self, src: &str, extra: &[usize]) -> usize { self.base + src.len() + extra.len() }
    pub fn pair_span(&self, idx: usize) -> usize { self.range_span(idx, idx) }
    pub fn range_span(&self, a: usize, b: usize) -> usize { self.base + a + b }
    pub fn using_self_state(&self, x: usize) -> usize { self.range_span(x, self.base) }
    pub fn with_strategy(&self, x: usize) -> usize { self.apply_with(x, |v| v + 1) }
    pub fn apply_with(&self, x: usize, f: impl Fn(usize) -> usize) -> usize { f(self.base + x) }
    fn private_default_forwarder(&self, x: usize) -> usize { self.range_span(x, 0) }
    fn keeps_self_state(&self, x: usize) -> usize { self.range_span(x, self.base) }
}
pub fn load_rules_from_dir(dir: &str) -> usize { load_rules_from_dir_with_registry(dir, &Registry::default()) }
pub fn load_rules_from_dir_with_registry(dir: &str, _reg: &Registry) -> usize { dir.len() }
pub fn load_embedded() -> usize { load_embedded_with_registry(&Registry::default()) }
pub fn load_embedded_with_registry(_reg: &Registry) -> usize { 42 }
fn private_free_forwarder(dir: &str) -> usize { load_rules_from_dir_with_registry(dir, &Registry::default()) }
pub fn main() -> usize { load_embedded_with_registry(&Registry::default()) }
mod bin_entry {
    pub fn run() -> usize { 1 }
    fn main() -> usize { run() }
}
";
        let codebase = parse_rust_codebase("src/loader.rs", src);
        let cfg = Config::default();
        let diags: Vec<(String, String)> = rules
            .iter()
            .flat_map(|r| r.check(&codebase, &cfg))
            .map(|d| (d.rule_id().to_string(), d.message().to_string()))
            .collect();

        let expected = [
            ("no-redundant-wrapper", "`load_rules`"),
            ("no-redundant-wrapper", "`from_config`"),
            ("no-redundant-wrapper", "`load_rules_from_dir`"),
            ("no-redundant-wrapper", "`load_embedded`"),
            ("no-private-forwarder", "`private_default_forwarder`"),
            ("no-private-forwarder", "`private_free_forwarder`"),
        ];
        assert_eq!(diags.len(), expected.len(), "unexpected: {diags:?}");
        for (rule, sym) in expected {
            assert!(
                diags
                    .iter()
                    .any(|(id, msg)| id == rule && msg.contains(sym)),
                "missing ({rule}, {sym}) in {diags:?}"
            );
        }
    }
}
