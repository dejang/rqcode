use crate::diagnostic::Severity;
use serde::{Deserialize, Serialize};

/// Configuration for the `no-barrel-module` policy under `[cohesion.boundaries]`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BarrelPolicyConfig {
    pub flag_barrel_modules: bool,
    pub allow_library_root_barrel: bool,
    pub max_barrel_related: usize,
}

impl Default for BarrelPolicyConfig {
    fn default() -> Self {
        Self {
            flag_barrel_modules: true,
            allow_library_root_barrel: false,
            max_barrel_related: 3,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct ModuleBoundaryConfig {
    pub flag_cyclic_modules: bool,
    pub flag_leaky_boundaries: bool,
    pub flag_stateless_impl_methods: bool,
    #[serde(flatten)]
    pub barrels: BarrelPolicyConfig,
}

impl Default for ModuleBoundaryConfig {
    fn default() -> Self {
        Self {
            flag_cyclic_modules: true,
            flag_leaky_boundaries: true,
            flag_stateless_impl_methods: true,
            barrels: BarrelPolicyConfig::default(),
        }
    }
}

/// Configuration for structural + conceptual type cohesion (`low-cohesion-type`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct TypeCohesionConfig {
    pub enabled: bool,
    pub min_fields: usize,
    pub min_methods: usize,
    pub min_field_using_methods: usize,
    pub min_component_methods: usize,
    pub max_accessor_nodes: usize,
    pub max_tcc: f64,
    pub max_conceptual_similarity: f64,
}

impl Default for TypeCohesionConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_fields: 4,
            min_methods: 5,
            min_field_using_methods: 4,
            min_component_methods: 2,
            max_accessor_nodes: 10,
            max_tcc: 0.50,
            max_conceptual_similarity: 0.20,
        }
    }
}

/// Configuration for structural + conceptual feature envy (`feature-envy`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FeatureEnvyConfig {
    pub enabled: bool,
    pub min_foreign_accesses: usize,
    pub max_foreign_providers: usize,
    pub max_laa: f64,
    pub min_conceptual_affinity: f64,
}

impl Default for FeatureEnvyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_foreign_accesses: 3,
            max_foreign_providers: 2,
            max_laa: 0.34,
            min_conceptual_affinity: 0.15,
        }
    }
}

/// Configuration for high fan-in + fan-out architectural crossings (`architectural-crossing`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct CrossingConfig {
    pub enabled: bool,
    pub min_graph_modules: usize,
    pub min_fan_in: usize,
    pub min_fan_out: usize,
    pub quantile: f64,
    pub max_related_edges: usize,
}

impl Default for CrossingConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            min_graph_modules: 12,
            min_fan_in: 4,
            min_fan_out: 4,
            quantile: 0.90,
            max_related_edges: 3,
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
    pub type_cohesion: TypeCohesionConfig,
    pub feature_envy: FeatureEnvyConfig,
    pub crossing: CrossingConfig,
    pub severity: Severity,
    pub advisory_severity: Severity,
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
            type_cohesion: TypeCohesionConfig::default(),
            feature_envy: FeatureEnvyConfig::default(),
            crossing: CrossingConfig::default(),
            severity: Severity::Error,
            advisory_severity: Severity::Warning,
        }
    }
}
