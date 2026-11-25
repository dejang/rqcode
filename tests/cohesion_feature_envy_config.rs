use rqcode::ast::SyntaxTree;
use rqcode::ast::codebase::Codebase;
use rqcode::config::{Config, FeatureEnvyConfig};
use rqcode::lang::LanguageFrontend;
use rqcode::lang::rust::{RustFrontend, rust_language_spec};
use rqcode::rules::cohesion::lexical::{LexicalCorpus, LexicalParams, cosine_similarity};
use std::error::Error;
use std::path::Path;

#[test]
fn feature_envy_config_defaults_and_toml_overrides_match_spec() -> Result<(), Box<dyn Error>> {
    let default_cfg = Config::default();
    let fe = &default_cfg.cohesion.feature_envy;
    assert_eq!(fe, &FeatureEnvyConfig::default());
    assert!(fe.enabled);
    assert_eq!(fe.min_foreign_accesses, 3);
    assert_eq!(fe.max_foreign_providers, 2);
    assert!((fe.max_laa - 0.34).abs() < 1e-12);
    assert!((fe.min_conceptual_affinity - 0.15).abs() < 1e-12);

    let full_toml = r"
[cohesion.feature_envy]
enabled = false
min_foreign_accesses = 5
max_foreign_providers = 1
max_laa = 0.25
min_conceptual_affinity = 0.30
";
    let parsed = Config::from_toml(full_toml)?;
    assert!(!parsed.cohesion.feature_envy.enabled);
    assert_eq!(parsed.cohesion.feature_envy.min_foreign_accesses, 5);
    assert_eq!(parsed.cohesion.feature_envy.max_foreign_providers, 1);
    assert!((parsed.cohesion.feature_envy.max_laa - 0.25).abs() < 1e-12);
    assert!((parsed.cohesion.feature_envy.min_conceptual_affinity - 0.30).abs() < 1e-12);

    let partial_toml = r"
[cohesion.feature_envy]
min_foreign_accesses = 4
";
    let partial = Config::from_toml(partial_toml)?;
    assert!(partial.cohesion.feature_envy.enabled);
    assert_eq!(partial.cohesion.feature_envy.min_foreign_accesses, 4);
    assert_eq!(partial.cohesion.feature_envy.max_foreign_providers, 2);
    assert!((partial.cohesion.feature_envy.max_laa - 0.34).abs() < 1e-12);
    assert!((partial.cohesion.feature_envy.min_conceptual_affinity - 0.15).abs() < 1e-12);
    Ok(())
}

#[test]
fn lexical_corpus_type_vector_excluding_includes_name_unread_fields_inherent_methods_only()
-> Result<(), Box<dyn Error>> {
    let src = r"pub trait AuditTrait {
    fn zeta_trait(&self) -> u64;
}

pub struct AlphaVault {
    beta_field: u64,
    omega_unread: u64,
}

impl AuditTrait for AlphaVault {
    fn zeta_trait(&self) -> u64 { 3 }
}

impl AlphaVault {
    pub fn gamma_other(&self) -> u64 { self.beta_field }
    pub fn delta_excluded(&self) -> u64 { 1 }
    #[cfg(test)]
    pub fn epsilon_test_only(&self) -> u64 { 2 }
}
";
    let tree = SyntaxTree::parse("src/sample.rs", "src/sample.rs", rust_language_spec(), src);
    let file = RustFrontend.extract(&tree, Path::new("."));
    let codebase = Codebase::build(vec![tree], vec![file]);
    let corpus = LexicalCorpus::build(&codebase, LexicalParams::default());

    let ty = codebase
        .types()
        .iter()
        .find(|t| t.decl.name == "AlphaVault")
        .ok_or("missing AlphaVault")?;
    let excluded = codebase
        .functions()
        .iter()
        .find(|f| f.decl.name == "delta_excluded")
        .ok_or("missing delta_excluded")?;

    let vec_excl = corpus.type_vector_excluding(&codebase, ty, Some(excluded.id));
    let vec_all = corpus.type_vector_excluding(&codebase, ty, None);

    for tok in ["alpha", "vault", "beta", "omega", "unread", "gamma"] {
        assert!(
            cosine_similarity(&vec_excl, &corpus.vector_from_tokens(&[tok])) > 0.0,
            "expected `{tok}` in type_vector_excluding"
        );
    }
    assert!(
        cosine_similarity(&vec_excl, &corpus.vector_from_tokens(&["delta"])) < 1e-12,
        "excluded method token must be omitted from v_own"
    );
    assert!(
        cosine_similarity(&vec_all, &corpus.vector_from_tokens(&["delta"])) > 0.0,
        "method token must be present when exclude_func is None"
    );
    for absent in ["epsilon", "zeta"] {
        assert!(
            cosine_similarity(&vec_all, &corpus.vector_from_tokens(&[absent])) < 1e-12,
            "non-inherent or test token `{absent}` must never enter type vector"
        );
    }
    Ok(())
}
