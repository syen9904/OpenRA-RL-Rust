//! Integration test: parse the Singles .oramap file used in our test replay.

use openra_data::oramap;

#[test]
fn parse_singles_map() {
    let data = std::fs::read(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/maps/singles.oramap")
    ).expect("Failed to read test map file");

    let map = oramap::parse(&data).expect("Failed to parse map");

    assert_eq!(map.title, "Singles");
    assert_eq!(map.tileset, "TEMPERAT");
    assert_eq!(map.map_size, (112, 54));
    assert_eq!(map.bounds, (2, 2, 108, 50));

    // Players
    eprintln!("Players: {}", map.players.len());
    for p in &map.players {
        eprintln!("  {} (playable={}, owns_world={}, faction={})",
            p.name, p.playable, p.owns_world, p.faction);
    }
    assert!(map.players.len() >= 3); // Neutral, Multi0, Multi1, Creeps

    // Actors
    eprintln!("\nActors: {}", map.actors.len());

    // Count actor types
    let mut types: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for a in &map.actors {
        *types.entry(&a.actor_type).or_default() += 1;
    }
    let mut sorted: Vec<_> = types.iter().collect();
    sorted.sort_by(|a, b| b.1.cmp(a.1));
    for (t, count) in &sorted {
        eprintln!("  {}: {}", t, count);
    }

    // Should have spawn points and mines
    assert!(map.actors.iter().any(|a| a.actor_type == "mpspawn"), "missing spawn points");
    assert!(map.actors.iter().any(|a| a.actor_type == "mine"), "missing mines");

    let spawn_count = map.actors.iter().filter(|a| a.actor_type == "mpspawn").count();
    eprintln!("\nSpawn points: {}", spawn_count);
    for (i, a) in map.actors.iter().enumerate() {
        if a.actor_type == "mpspawn" {
            eprintln!("  mpspawn #{}: location=({},{}) owner={}", i, a.location.0, a.location.1, a.owner);
        }
    }
    assert_eq!(spawn_count, 2);
}
