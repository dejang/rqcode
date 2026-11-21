use super::AstGrepYamlRule;
use crate::ast::LanguageSpec;
use crate::lang::LanguageRegistry;
use ast_grep_config::{GlobalRules, RuleConfig, SerializableRuleConfig, from_multiple};
use ast_grep_core::language::Language;
use ast_grep_core::matcher::{Pattern, PatternBuilder, PatternError};
use serde::Deserialize;
use std::fmt;
use std::path::{Path, PathBuf};
use thiserror::Error;

include!(concat!(env!("OUT_DIR"), "/embedded_ast_grep_rules.rs"));

/// Placeholder language type used solely while deserializing raw ast-grep YAML documents
/// before resolving each rule's `language:` string through [`LanguageRegistry`].
#[derive(Clone, Deserialize)]
#[serde(transparent)]
struct YamlLangName(String);

impl Language for YamlLangName {
    fn kind_to_id(&self, _kind: &str) -> u16 {
        0
    }

    fn field_to_id(&self, _field: &str) -> Option<u16> {
        None
    }

    fn build_pattern(&self, _builder: &PatternBuilder) -> Result<Pattern, PatternError> {
        Err(PatternError::NoContent(String::new()))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleOrigin {
    Embedded(&'static str),
    File(PathBuf),
    Inline,
}

impl RuleOrigin {
    /// Turn an ast-grep YAML string (single or multi-document) into runnable rules, resolving
    /// each rule's `language` name via `registry`.
    ///
    /// # Errors
    /// Returns [`AstGrepRuleError::ParseYaml`] if the YAML is malformed or names an unsupported language,
    /// or [`AstGrepRuleError::InvalidGlob`] if a rule's file globs fail to compile.
    pub fn load_rules(
        &self,
        yaml: &str,
        registry: &LanguageRegistry,
    ) -> Result<Vec<AstGrepYamlRule>, AstGrepRuleError> {
        let mut rules = Vec::new();
        self.load_rules_into(yaml, registry, &mut rules)?;
        Ok(rules)
    }

    /// Parse an ast-grep YAML string and append compiled rules into `out`, resolving languages via `registry`.
    ///
    /// # Errors
    /// Returns [`AstGrepRuleError::ParseYaml`] if the YAML is malformed or names an unsupported language,
    /// or [`AstGrepRuleError::InvalidGlob`] if a rule's file globs fail to compile.
    pub fn load_rules_into(
        &self,
        yaml: &str,
        registry: &LanguageRegistry,
        out: &mut Vec<AstGrepYamlRule>,
    ) -> Result<(), AstGrepRuleError> {
        let raw_docs =
            from_multiple::<SerializableRuleConfig<YamlLangName>>(yaml).map_err(|err| {
                AstGrepRuleError::ParseYaml {
                    origin: self.clone(),
                    message: err.to_string(),
                }
            })?;

        let globals = GlobalRules::default();
        for raw in raw_docs {
            let lang_name = raw.language.0;
            let Some(language) = registry.resolve_language(&lang_name) else {
                return Err(AstGrepRuleError::ParseYaml {
                    origin: self.clone(),
                    message: format!("unsupported ast-grep language `{lang_name}`"),
                });
            };
            let mapped: SerializableRuleConfig<LanguageSpec> = SerializableRuleConfig {
                core: raw.core,
                fix: raw.fix,
                rewriters: raw.rewriters,
                id: raw.id,
                language,
                message: raw.message,
                note: raw.note,
                severity: raw.severity,
                labels: raw.labels,
                files: raw.files,
                ignores: raw.ignores,
                url: raw.url,
                metadata: raw.metadata,
                unknown_keys: raw.unknown_keys,
            };
            let cfg = RuleConfig::try_from(mapped, &globals).map_err(|err| {
                AstGrepRuleError::ParseYaml {
                    origin: self.clone(),
                    message: err.to_string(),
                }
            })?;
            out.push(AstGrepYamlRule::from_rule_config(cfg, self.clone())?);
        }
        Ok(())
    }
}

impl fmt::Display for RuleOrigin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Embedded(name) => write!(f, "embedded:{name}"),
            Self::File(path) => write!(f, "{}", path.display()),
            Self::Inline => f.write_str("<inline-yaml>"),
        }
    }
}

#[derive(Debug, Error)]
pub enum AstGrepRuleError {
    #[error("ast-grep rules directory does not exist: {0}")]
    MissingRulesDir(PathBuf),
    #[error("ast-grep rules path is not a directory: {0}")]
    NotADirectory(PathBuf),
    #[error("failed to read ast-grep rule path {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse ast-grep YAML rule from {origin}: {message}")]
    ParseYaml { origin: RuleOrigin, message: String },
    #[error("invalid file glob in ast-grep rule `{rule_id}` from {origin}: {message}")]
    InvalidGlob {
        origin: RuleOrigin,
        rule_id: String,
        message: String,
    },
}

/// Read a single ast-grep YAML file from disk and compile all rule documents inside it.
///
/// # Errors
/// Returns [`AstGrepRuleError::Io`] if `path` cannot be read, or a YAML/glob error if parsing fails.
pub fn load_rules_from_file(
    path: &Path,
    registry: &LanguageRegistry,
) -> Result<Vec<AstGrepYamlRule>, AstGrepRuleError> {
    let content = std::fs::read_to_string(path).map_err(|source| AstGrepRuleError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    RuleOrigin::File(path.to_path_buf()).load_rules(&content, registry)
}

/// Recursively load all `.yml`/`.yaml` ast-grep rule files from `dir` in deterministic path order,
/// resolving rule languages via `registry`.
///
/// # Errors
/// Returns [`AstGrepRuleError::MissingRulesDir`] if `dir` does not exist, [`AstGrepRuleError::NotADirectory`]
/// if `dir` is not a directory, or I/O and parse errors for individual rule files.
pub fn load_rules_from_dir(
    dir: &Path,
    registry: &LanguageRegistry,
) -> Result<Vec<AstGrepYamlRule>, AstGrepRuleError> {
    if !dir.exists() {
        return Err(AstGrepRuleError::MissingRulesDir(dir.to_path_buf()));
    }
    if !dir.is_dir() {
        return Err(AstGrepRuleError::NotADirectory(dir.to_path_buf()));
    }

    let mut yaml_files = Vec::new();
    collect_yaml_files(dir, &mut yaml_files)?;
    yaml_files.sort();

    let mut rules = Vec::new();
    for file in yaml_files {
        let content = std::fs::read_to_string(&file).map_err(|source| AstGrepRuleError::Io {
            path: file.clone(),
            source,
        })?;
        RuleOrigin::File(file).load_rules_into(&content, registry, &mut rules)?;
    }
    Ok(rules)
}

/// Load all built-in ast-grep YAML rules embedded into the binary, resolving languages via `registry`.
///
/// # Errors
/// Returns [`AstGrepRuleError`] if any embedded YAML rule fails to parse.
pub fn load_embedded_rules(
    registry: &LanguageRegistry,
) -> Result<Vec<AstGrepYamlRule>, AstGrepRuleError> {
    let mut rules = Vec::new();
    for &(name, yaml) in EMBEDDED_AST_GREP_RULES {
        RuleOrigin::Embedded(name).load_rules_into(yaml, registry, &mut rules)?;
    }
    Ok(rules)
}

fn collect_yaml_files(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), AstGrepRuleError> {
    let entries = std::fs::read_dir(dir).map_err(|source| AstGrepRuleError::Io {
        path: dir.to_path_buf(),
        source,
    })?;

    for entry in entries {
        let entry = entry.map_err(|source| AstGrepRuleError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        if path.is_dir() {
            collect_yaml_files(&path, out)?;
        } else if path.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| {
                    ext.eq_ignore_ascii_case("yml") || ext.eq_ignore_ascii_case("yaml")
                })
        {
            out.push(path);
        }
    }
    Ok(())
}
