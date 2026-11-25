use rqcode::config::{Config, CrossingConfig};
use rqcode::rules::cohesion::CohesionRuleId;
use std::error::Error;

#[test]
fn crossing_config_defaults_and_toml_overrides_match_spec() -> Result<(), Box<dyn Error>> {
    assert_eq!(
        CohesionRuleId::ArchitecturalCrossing.as_str(),
        "architectural-crossing"
    );

    let default_cfg = Config::default();
    let cr = &default_cfg.cohesion.crossing;
    assert_eq!(cr, &CrossingConfig::default());
    assert!(cr.enabled);
    assert_eq!(cr.min_graph_modules, 12);
    assert_eq!(cr.min_fan_in, 4);
    assert_eq!(cr.min_fan_out, 4);
    assert!((cr.quantile - 0.90).abs() < 1e-12);
    assert_eq!(cr.max_related_edges, 3);

    let full_toml = r"
[cohesion.crossing]
enabled = false
min_graph_modules = 8
min_fan_in = 5
min_fan_out = 6
quantile = 0.85
max_related_edges = 2
";
    let parsed = Config::from_toml(full_toml)?;
    assert!(!parsed.cohesion.crossing.enabled);
    assert_eq!(parsed.cohesion.crossing.min_graph_modules, 8);
    assert_eq!(parsed.cohesion.crossing.min_fan_in, 5);
    assert_eq!(parsed.cohesion.crossing.min_fan_out, 6);
    assert!((parsed.cohesion.crossing.quantile - 0.85).abs() < 1e-12);
    assert_eq!(parsed.cohesion.crossing.max_related_edges, 2);

    let partial_toml = r"
[cohesion.crossing]
min_fan_in = 5
max_related_edges = 1
";
    let partial = Config::from_toml(partial_toml)?;
    assert!(partial.cohesion.crossing.enabled);
    assert_eq!(partial.cohesion.crossing.min_graph_modules, 12);
    assert_eq!(partial.cohesion.crossing.min_fan_in, 5);
    assert_eq!(partial.cohesion.crossing.min_fan_out, 4);
    assert!((partial.cohesion.crossing.quantile - 0.90).abs() < 1e-12);
    assert_eq!(partial.cohesion.crossing.max_related_edges, 1);
    Ok(())
}
