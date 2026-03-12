//! Game rules loader — parses OpenRA's YAML rules into structured data.
//!
//! Loads actor definitions, weapon definitions, and other game rules from
//! the mod's rules/*.yaml files using the MiniYAML parser.
//!
//! Reference: OpenRA.Game/GameRules/Ruleset.cs, ActorInfo.cs

use crate::miniyaml::{self, MiniYamlNode};
use std::collections::BTreeMap;
use std::path::Path;

/// A trait definition on an actor (e.g., Health, Mobile, Armament@PRIMARY).
#[derive(Debug, Clone)]
pub struct TraitInfo {
    /// Trait type name (e.g., "Health", "Armament", "Mobile").
    pub trait_name: String,
    /// Instance name for named traits (e.g., "PRIMARY" for Armament@PRIMARY).
    pub instance_name: Option<String>,
    /// Key-value parameters (e.g., "HP" -> "150000").
    pub params: BTreeMap<String, String>,
    /// Nested child nodes (for complex params like warhead definitions).
    pub children: Vec<MiniYamlNode>,
}

impl TraitInfo {
    /// Get a parameter value by key.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(|s| s.as_str())
    }

    /// Get a parameter as i32.
    pub fn get_i32(&self, key: &str) -> Option<i32> {
        self.get(key).and_then(|v| v.parse().ok())
    }

    /// Get a parameter as bool.
    pub fn get_bool(&self, key: &str) -> Option<bool> {
        self.get(key).map(|v| v.eq_ignore_ascii_case("true") || v == "True")
    }
}

/// An actor definition (e.g., E1, FACT, MCV).
#[derive(Debug, Clone)]
pub struct ActorInfo {
    pub name: String,
    pub traits: Vec<TraitInfo>,
}

impl ActorInfo {
    /// Find a trait by type name (first match).
    pub fn trait_info(&self, name: &str) -> Option<&TraitInfo> {
        self.traits.iter().find(|t| t.trait_name == name)
    }

    /// Find a trait by type name and instance (e.g., "Armament", "PRIMARY").
    pub fn trait_instance(&self, name: &str, instance: &str) -> Option<&TraitInfo> {
        self.traits.iter().find(|t| {
            t.trait_name == name && t.instance_name.as_deref() == Some(instance)
        })
    }

    /// Get all traits of a given type.
    pub fn traits_of(&self, name: &str) -> Vec<&TraitInfo> {
        self.traits.iter().filter(|t| t.trait_name == name).collect()
    }

    /// Check if this actor has a specific trait.
    pub fn has_trait(&self, name: &str) -> bool {
        self.traits.iter().any(|t| t.trait_name == name)
    }
}

/// A weapon definition.
#[derive(Debug, Clone)]
pub struct WeaponInfo {
    pub name: String,
    pub params: BTreeMap<String, String>,
    pub children: Vec<MiniYamlNode>,
}

impl WeaponInfo {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.params.get(key).map(|s| s.as_str())
    }

    pub fn get_i32(&self, key: &str) -> Option<i32> {
        self.get(key).and_then(|v| v.parse().ok())
    }
}

/// The complete game ruleset.
#[derive(Debug, Clone)]
pub struct Ruleset {
    pub actors: BTreeMap<String, ActorInfo>,
    pub weapons: BTreeMap<String, WeaponInfo>,
}

impl Ruleset {
    /// Get an actor definition by name.
    pub fn actor(&self, name: &str) -> Option<&ActorInfo> {
        self.actors.get(name)
    }

    /// Get a weapon definition by name.
    pub fn weapon(&self, name: &str) -> Option<&WeaponInfo> {
        self.weapons.get(name)
    }
}

/// Convert a resolved MiniYamlNode into an ActorInfo.
fn node_to_actor(node: &MiniYamlNode) -> ActorInfo {
    let mut traits = Vec::new();

    for child in &node.children {
        let (trait_name, instance_name) = if let Some(at_pos) = child.key.find('@') {
            (
                child.key[..at_pos].to_string(),
                Some(child.key[at_pos + 1..].to_string()),
            )
        } else {
            (child.key.clone(), None)
        };

        let mut params = BTreeMap::new();
        for grandchild in &child.children {
            if grandchild.children.is_empty() {
                params.insert(grandchild.key.clone(), grandchild.value.clone());
            }
        }

        traits.push(TraitInfo {
            trait_name,
            instance_name,
            params,
            children: child.children.clone(),
        });
    }

    ActorInfo {
        name: node.key.clone(),
        traits,
    }
}

/// Convert a resolved MiniYamlNode into a WeaponInfo.
fn node_to_weapon(node: &MiniYamlNode) -> WeaponInfo {
    let mut params = BTreeMap::new();
    for child in &node.children {
        if child.children.is_empty() {
            params.insert(child.key.clone(), child.value.clone());
        }
    }

    WeaponInfo {
        name: node.key.clone(),
        params,
        children: node.children.clone(),
    }
}

/// Load a ruleset from a mod directory (e.g., "/path/to/OpenRA/mods/ra").
///
/// Reads rules/*.yaml and weapons/*.yaml, merges them with defaults,
/// resolves inheritance, and returns a complete Ruleset.
pub fn load_ruleset(mod_dir: &Path) -> std::io::Result<Ruleset> {
    let rules_dir = mod_dir.join("rules");
    let weapons_dir = mod_dir.join("weapons");

    // Load and merge all rules files in a deterministic order.
    // defaults.yaml must come first, then other files alphabetically.
    let rule_files = &[
        "defaults.yaml",
        "player.yaml",
        "world.yaml",
        "infantry.yaml",
        "vehicles.yaml",
        "aircraft.yaml",
        "ships.yaml",
        "structures.yaml",
        "decoration.yaml",
        "misc.yaml",
        "civilian.yaml",
        "fakes.yaml",
        "husks.yaml",
    ];

    let mut rule_sources = Vec::new();
    for filename in rule_files {
        let path = rules_dir.join(filename);
        if path.exists() {
            rule_sources.push(std::fs::read_to_string(&path)?);
        }
    }

    let source_refs: Vec<&str> = rule_sources.iter().map(|s| s.as_str()).collect();
    let merged = miniyaml::parse_and_merge(&source_refs);
    let resolved = miniyaml::resolve_inherits(merged);

    let mut actors = BTreeMap::new();
    for node in &resolved {
        actors.insert(node.key.clone(), node_to_actor(node));
    }

    // Load weapons
    let mut weapons = BTreeMap::new();
    if weapons_dir.exists() {
        let mut weapon_sources = Vec::new();
        if let Ok(entries) = std::fs::read_dir(&weapons_dir) {
            let mut filenames: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map_or(false, |ext| ext == "yaml"))
                .map(|e| e.path())
                .collect();
            filenames.sort();
            for path in &filenames {
                weapon_sources.push(std::fs::read_to_string(path)?);
            }
        }
        let weapon_refs: Vec<&str> = weapon_sources.iter().map(|s| s.as_str()).collect();
        let weapon_merged = miniyaml::parse_and_merge(&weapon_refs);
        let weapon_resolved = miniyaml::resolve_inherits(weapon_merged);
        for node in &weapon_resolved {
            weapons.insert(node.key.clone(), node_to_weapon(node));
        }
    }

    Ok(Ruleset { actors, weapons })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_ra_ruleset() {
        let mod_dir = Path::new("/Users/berta/Projects/OpenRA/mods/ra");
        if !mod_dir.exists() {
            eprintln!("Skipping: OpenRA mod dir not found");
            return;
        }
        let ruleset = load_ruleset(mod_dir).unwrap();

        // Basic sanity checks
        assert!(ruleset.actors.len() > 50, "Expected 50+ actors, got {}", ruleset.actors.len());
        eprintln!("Loaded {} actors, {} weapons", ruleset.actors.len(), ruleset.weapons.len());

        // Check MCV
        let mcv = ruleset.actor("MCV").expect("MCV not found");
        assert!(mcv.has_trait("Mobile"), "MCV should have Mobile");
        assert!(mcv.has_trait("Health"), "MCV should have Health");
        let hp = mcv.trait_info("Health").unwrap().get_i32("HP");
        assert_eq!(hp, Some(60000), "MCV HP should be 60000");

        // Check FACT
        let fact = ruleset.actor("FACT").expect("FACT not found");
        assert!(fact.has_trait("Building"), "FACT should have Building");
        let hp = fact.trait_info("Health").unwrap().get_i32("HP");
        assert_eq!(hp, Some(150000), "FACT HP should be 150000");

        // Check POWR cost
        let powr = ruleset.actor("POWR").expect("POWR not found");
        let cost = powr.trait_info("Valued").unwrap().get_i32("Cost");
        assert_eq!(cost, Some(300), "POWR cost should be 300");

        // Check E1 (Rifleman)
        let e1 = ruleset.actor("E1").expect("E1 not found");
        assert!(e1.has_trait("Health"), "E1 should have Health");
        assert!(e1.has_trait("Mobile"), "E1 should have Mobile");
    }

    #[test]
    fn load_ra_weapons() {
        let mod_dir = Path::new("/Users/berta/Projects/OpenRA/mods/ra");
        if !mod_dir.exists() {
            eprintln!("Skipping: OpenRA mod dir not found");
            return;
        }
        let ruleset = load_ruleset(mod_dir).unwrap();
        assert!(ruleset.weapons.len() > 10, "Expected 10+ weapons, got {}", ruleset.weapons.len());
        eprintln!("Weapons: {:?}", ruleset.weapons.keys().collect::<Vec<_>>());
    }
}
