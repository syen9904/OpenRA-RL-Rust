//! Test MiniYAML parser against real OpenRA rule files.

use openra_data::miniyaml;

const OPENRA_MODS: &str = "/Users/berta/Projects/OpenRA/mods/ra/rules";

fn read_rules(filename: &str) -> String {
    let path = format!("{}/{}", OPENRA_MODS, filename);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("Failed to read {}: {}", path, e))
}

#[test]
fn parse_defaults_yaml() {
    let text = read_rules("defaults.yaml");
    let nodes = miniyaml::parse(&text);
    // defaults.yaml has many ^Abstract nodes
    assert!(nodes.len() > 10, "Expected many top-level nodes, got {}", nodes.len());

    // Check ^ExistsInWorld exists
    let eiw = nodes.iter().find(|n| n.key == "^ExistsInWorld");
    assert!(eiw.is_some(), "^ExistsInWorld not found");
    let eiw = eiw.unwrap();
    assert!(eiw.children.iter().any(|c| c.key == "AppearsOnRadar"),
        "^ExistsInWorld should have AppearsOnRadar");
}

#[test]
fn parse_infantry_yaml() {
    let text = read_rules("infantry.yaml");
    let nodes = miniyaml::parse(&text);
    assert!(nodes.len() > 5, "Expected several infantry, got {}", nodes.len());

    // E1 should exist
    let e1 = nodes.iter().find(|n| n.key == "E1");
    assert!(e1.is_some(), "E1 (Rifleman) not found");
}

#[test]
fn parse_structures_yaml() {
    let text = read_rules("structures.yaml");
    let nodes = miniyaml::parse(&text);

    // FACT (Construction Yard) should exist
    let fact = nodes.iter().find(|n| n.key == "FACT");
    assert!(fact.is_some(), "FACT not found");
    let fact = fact.unwrap();

    // FACT should have Health trait with HP
    let health = fact.children.iter().find(|c| c.key == "Health");
    assert!(health.is_some(), "FACT should have Health");
    let hp = health.unwrap().child_value("HP");
    assert_eq!(hp, Some("150000"), "FACT HP should be 150000");
}

#[test]
fn resolve_infantry_with_defaults() {
    let defaults = read_rules("defaults.yaml");
    let infantry = read_rules("infantry.yaml");

    let merged = miniyaml::parse_and_merge(&[&defaults, &infantry]);
    let resolved = miniyaml::resolve_inherits(merged);

    // E1 should have inherited traits from its parents
    let e1 = resolved.iter().find(|n| n.key == "E1");
    assert!(e1.is_some(), "E1 not found after resolve");
    let e1 = e1.unwrap();

    // E1 inherits from ^Soldier which inherits from ^Infantry etc.
    // Should have Health, Mobile, and other traits
    let trait_names: Vec<&str> = e1.children.iter().map(|c| c.key.as_str()).collect();
    eprintln!("E1 traits: {:?}", trait_names);
    assert!(trait_names.iter().any(|t| *t == "Health"), "E1 should have Health");
}

#[test]
fn resolve_structures_with_defaults() {
    let defaults = read_rules("defaults.yaml");
    let structures = read_rules("structures.yaml");

    let merged = miniyaml::parse_and_merge(&[&defaults, &structures]);
    let resolved = miniyaml::resolve_inherits(merged);

    // POWR (Power Plant) - check cost
    let powr = resolved.iter().find(|n| n.key == "POWR");
    assert!(powr.is_some(), "POWR not found");
    let powr = powr.unwrap();

    let valued = powr.children.iter().find(|c| c.key == "Valued");
    assert!(valued.is_some(), "POWR should have Valued");
    let cost = valued.unwrap().child_value("Cost");
    assert_eq!(cost, Some("300"), "POWR cost should be 300");
}
