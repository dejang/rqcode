use crate::ast::similarity::{EligibilityConfig, SimilarityParams};
use crate::diagnostic::Severity;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read config file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse config TOML: {0}")]
    Parse(#[from] toml::de::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub exclude: Vec<String>,
    pub similarity: SimilarityConfig,
    pub cohesion: CohesionConfig,
    pub test_coupling: TestCouplingConfig,
    pub ast_grep: AstGrepConfig,
    #[serde(skip)]
    base_dir: Option<PathBuf>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            exclude: vec![".git".to_string()],
            similarity: SimilarityConfig::default(),
            cohesion: CohesionConfig::default(),
            test_coupling: TestCouplingConfig::default(),
            ast_grep: AstGrepConfig::default(),
            base_dir: None,
        }
    }
}

impl Config {
    /// Load `rqcode.toml` from `root` (or its parent if `root` is a file), otherwise return defaults.
    ///
    /// # Errors
    /// Returns [`ConfigError`] if an existing `rqcode.toml` cannot be read or contains invalid TOML.
    pub fn load_from_dir(root: &Path) -> Result<Self, ConfigError> {
        let base_dir = if root.is_file() {
            root.parent().unwrap_or(root)
        } else {
            root
        };
        let candidate = base_dir.join("rqcode.toml");
        if candidate.is_file() {
            Self::load_file(&candidate)
        } else {
            Ok(Self::default().with_config_dir(base_dir))
        }
    }

    /// Load configuration from a specific TOML file path.
    ///
    /// # Errors
    /// Returns [`ConfigError::Io`] if `path` cannot be read or [`ConfigError::Parse`] if the TOML is invalid.
    pub fn load_file(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path).map_err(|source| ConfigError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        let mut config = Self::from_toml(&content)?;
        if let Some(parent) = path.parent() {
            config.base_dir = Some(if parent.as_os_str().is_empty() {
                PathBuf::from(".")
            } else {
                parent.to_path_buf()
            });
        }
        Ok(config)
    }

    /// Parse configuration from a TOML string.
    ///
    /// # Errors
    /// Returns [`ConfigError::Parse`] if `content` is not valid TOML matching [`Config`].
    pub fn from_toml(content: &str) -> Result<Self, ConfigError> {
        Ok(toml::from_str(content)?)
    }

    #[must_use]
    pub fn config_dir(&self) -> Option<&Path> {
        self.base_dir.as_deref()
    }

    #[must_use]
    pub fn with_config_dir(mut self, dir: impl Into<PathBuf>) -> Self {
        self.base_dir = Some(dir.into());
        self
    }

    /// Resolve the configured `ast_grep.rules_dir` relative to the config file's directory
    /// (or `fallback_root` if the config was constructed in memory).
    #[must_use]
    pub fn resolve_ast_grep_rules_dir(&self, fallback_root: &Path) -> Option<PathBuf> {
        let dir = self.ast_grep.rules_dir.as_ref()?;
        if dir.is_absolute() {
            return Some(dir.clone());
        }
        let base = self.base_dir.as_deref().unwrap_or_else(|| {
            if fallback_root.is_file() {
                fallback_root.parent().unwrap_or(fallback_root)
            } else {
                fallback_root
            }
        });
        Some(base.join(dir))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SimilarityConfig {
    pub enabled: bool,
    pub function_threshold: f64,
    pub test_threshold: f64,
    /// Minimum folded normalized AST node count required before comparing a function or test.
    pub min_tokens: usize,
    /// Minimum distinct structural kinds / operators / callees (`TKS`) required for eligibility.
    pub min_distinct_kinds: usize,
    pub severity: Severity,
}

impl Default for SimilarityConfig {
    fn default() -> Self {
        let eligibility = EligibilityConfig::default();
        Self {
            enabled: true,
            function_threshold: 0.85,
            test_threshold: 0.88,
            min_tokens: eligibility.min_nodes,
            min_distinct_kinds: eligibility.min_distinct_kinds,
            severity: Severity::Error,
        }
    }
}

impl SimilarityConfig {
    #[must_use]
    pub fn to_params(&self) -> SimilarityParams {
        SimilarityParams {
            eligibility: EligibilityConfig {
                min_nodes: self.min_tokens,
                min_distinct_kinds: self.min_distinct_kinds,
            },
            ..SimilarityParams::default()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModuleBoundaryConfig {
    pub flag_cyclic_modules: bool,
    pub flag_leaky_boundaries: bool,
    pub flag_stateless_impl_methods: bool,
}

impl Default for ModuleBoundaryConfig {
    fn default() -> Self {
        Self {
            flag_cyclic_modules: true,
            flag_leaky_boundaries: true,
            flag_stateless_impl_methods: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CohesionConfig {
    pub enabled: bool,
    pub min_external_functions: usize,
    pub min_external_modules: usize,
    pub flag_external_field_mutation: bool,
    pub flag_test_scatter: bool,
    pub boundaries: ModuleBoundaryConfig,
    pub severity: Severity,
}

impl Default for CohesionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_external_functions: 2,
            min_external_modules: 2,
            flag_external_field_mutation: true,
            flag_test_scatter: true,
            boundaries: ModuleBoundaryConfig::default(),
            severity: Severity::Error,
        }
    }
}

/// Controls whether public items are checked for test-only usage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum PublicApiScope {
    /// Only check public items in binary-only packages (packages with a binary entrypoint and no library root).
    #[default]
    BinaryOnly,
    /// Check public items in library packages as well.
    All,
}

/// Toggle flags for test-only production code and private-member coupling checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TestCouplingCoreFlags {
    pub flag_test_only_production_api: bool,
    pub flag_test_only_production_field: bool,
    pub flag_private_test_coupling: bool,
}

impl Default for TestCouplingCoreFlags {
    fn default() -> Self {
        Self {
            flag_test_only_production_api: true,
            flag_test_only_production_field: true,
            flag_private_test_coupling: true,
        }
    }
}

/// Toggle flags for public-surface boundary and test-driven overexposure checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TestCouplingSurfaceFlags {
    pub flag_test_bypasses_public_surface: bool,
    pub flag_overexposed_for_tests: bool,
}

impl Default for TestCouplingSurfaceFlags {
    fn default() -> Self {
        Self {
            flag_test_bypasses_public_surface: true,
            flag_overexposed_for_tests: true,
        }
    }
}

/// Toggle flags for production test hooks, encapsulation escapes, and representation assertions.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TestCouplingHookFlags {
    pub flag_production_test_hook: bool,
    pub flag_test_encapsulation_escape: bool,
    pub flag_test_asserts_internal_representation: bool,
}

impl Default for TestCouplingHookFlags {
    fn default() -> Self {
        Self {
            flag_production_test_hook: true,
            flag_test_encapsulation_escape: true,
            flag_test_asserts_internal_representation: true,
        }
    }
}

/// Toggle flags for internal-collaborator mocking, SUT spying, and interaction-only tests.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TestCouplingMockFlags {
    pub flag_test_mocks_internal_collaborator: bool,
    pub flag_test_spies_on_sut: bool,
    pub flag_interaction_only_test: bool,
}

impl Default for TestCouplingMockFlags {
    fn default() -> Self {
        Self {
            flag_test_mocks_internal_collaborator: true,
            flag_test_spies_on_sut: true,
            flag_interaction_only_test: true,
        }
    }
}

/// Toggle flags for mock-only abstraction checks.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TestCouplingAbstractionFlags {
    pub flag_mock_only_abstraction: bool,
}

impl Default for TestCouplingAbstractionFlags {
    fn default() -> Self {
        Self {
            flag_mock_only_abstraction: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TestCouplingConfig {
    pub enabled: bool,
    #[serde(flatten)]
    pub core: TestCouplingCoreFlags,
    #[serde(flatten)]
    pub surface: TestCouplingSurfaceFlags,
    #[serde(flatten)]
    pub hooks: TestCouplingHookFlags,
    #[serde(flatten)]
    pub mocking: TestCouplingMockFlags,
    #[serde(flatten)]
    pub abstractions: TestCouplingAbstractionFlags,
    pub public_api_scope: PublicApiScope,
    pub severity: Severity,
}

impl Default for TestCouplingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            core: TestCouplingCoreFlags::default(),
            surface: TestCouplingSurfaceFlags::default(),
            hooks: TestCouplingHookFlags::default(),
            mocking: TestCouplingMockFlags::default(),
            abstractions: TestCouplingAbstractionFlags::default(),
            public_api_scope: PublicApiScope::BinaryOnly,
            severity: Severity::Error,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AstGrepConfig {
    pub enabled: bool,
    /// Whether built-in ast-grep YAML rules embedded in the binary are active.
    pub builtin_rules: bool,
    /// Optional folder containing project-specific ast-grep YAML rules to load on top of built-ins.
    pub rules_dir: Option<PathBuf>,
    /// Rule IDs (embedded or custom) explicitly turned off via configuration.
    pub disabled_rules: Vec<String>,
    /// Optional inline TOML pattern rules.
    pub rules: Vec<PatternRuleConfig>,
}

impl Default for AstGrepConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            builtin_rules: true,
            rules_dir: None,
            disabled_rules: Vec::new(),
            rules: Vec::new(),
        }
    }
}

impl AstGrepConfig {
    #[must_use]
    pub fn is_rule_disabled(&self, rule_id: &str) -> bool {
        self.disabled_rules.iter().any(|id| id == rule_id)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PatternRuleConfig {
    pub id: String,
    pub language: String,
    pub pattern: String,
    pub message: String,
    pub suggestion: Option<String>,
    pub severity: Severity,
    pub skip_tests: bool,
}

impl Default for PatternRuleConfig {
    fn default() -> Self {
        Self {
            id: String::new(),
            language: String::new(),
            pattern: String::new(),
            message: String::new(),
            suggestion: None,
            severity: Severity::Error,
            skip_tests: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_partial_toml_preserving_defaults() {
        let toml = r"
            [similarity]
            function_threshold = 0.92
            min_tokens = 40
        ";

        let config = Config::from_toml(toml).expect("valid partial TOML");
        assert!((config.similarity.function_threshold - 0.92).abs() < f64::EPSILON);
        assert_eq!(config.similarity.min_tokens, 40);
        assert_eq!(config.similarity.min_distinct_kinds, 4);
        assert!(config.cohesion.enabled);
        assert!(config.test_coupling.enabled);
        assert!(config.test_coupling.core.flag_test_only_production_api);
        assert!(config.test_coupling.core.flag_test_only_production_field);
        assert_eq!(
            config.test_coupling.public_api_scope,
            PublicApiScope::BinaryOnly
        );
        assert!(config.ast_grep.enabled);
        assert!(config.ast_grep.builtin_rules);
        assert!(config.ast_grep.rules_dir.is_none());
        assert_eq!(config.ast_grep.disabled_rules, Vec::<String>::new());
    }

    #[test]
    fn parses_flattened_test_coupling_flags_from_toml() {
        let toml = r"
            [test_coupling]
            flag_test_only_production_field = false
        ";
        let config = Config::from_toml(toml).expect("valid test_coupling config");
        assert!(config.test_coupling.enabled);
        assert!(config.test_coupling.core.flag_test_only_production_api);
        assert!(!config.test_coupling.core.flag_test_only_production_field);
        assert!(config.test_coupling.core.flag_private_test_coupling);
    }

    #[test]
    fn parses_ast_grep_rules_dir_and_disabling_options() {
        let toml = r#"
            [ast_grep]
            builtin_rules = false
            rules_dir = "custom/rules"
            disabled_rules = ["no-dbg-macro", "no-unwrap"]
        "#;

        let config = Config::from_toml(toml).expect("valid ast_grep config");
        assert!(!config.ast_grep.builtin_rules);
        assert_eq!(
            config.ast_grep.rules_dir.as_deref(),
            Some(Path::new("custom/rules"))
        );
        assert!(config.ast_grep.is_rule_disabled("no-dbg-macro"));
        assert!(config.ast_grep.is_rule_disabled("no-unwrap"));
        assert!(!config.ast_grep.is_rule_disabled("no-todo-macro"));
    }

    #[test]
    fn resolves_relative_rules_dir_against_config_file_directory() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cfg_dir = temp.path().join("config");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        let cfg_file = cfg_dir.join("rqcode.toml");
        std::fs::write(&cfg_file, "[ast_grep]\nrules_dir = \"../ast-rules\"\n").unwrap();

        let cfg = Config::load_file(&cfg_file).expect("load config file");
        assert_eq!(cfg.config_dir(), Some(cfg_dir.as_path()));
        assert_eq!(
            cfg.resolve_ast_grep_rules_dir(Path::new("/unrelated")),
            Some(cfg_dir.join("../ast-rules"))
        );
    }

    #[test]
    fn rejects_malformed_toml() {
        let err = Config::from_toml("[similarity\nnot valid");
        assert!(matches!(err, Err(ConfigError::Parse(_))));
    }

    #[test]
    fn returns_defaults_when_config_file_is_absent() {
        let temp = tempfile::tempdir().expect("tempdir");
        let cfg = Config::load_from_dir(temp.path()).expect("default load");
        assert_eq!(cfg.cohesion.min_external_modules, 2);
        assert!(cfg.test_coupling.enabled);
        assert!(cfg.ast_grep.builtin_rules);
    }
}
