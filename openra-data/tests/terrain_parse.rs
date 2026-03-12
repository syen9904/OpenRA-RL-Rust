//! Test terrain tile parsing from map.bin.

use openra_data::oramap;

#[test]
fn parse_terrain_tiles() {
    let map_data = std::fs::read(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/maps/singles.oramap")
    ).unwrap();
    let map = oramap::parse(&map_data).unwrap();

    // Map should be 112x54
    assert_eq!(map.map_size, (112, 54));

    // Tiles should be populated
    assert_eq!(map.tiles.len(), 54, "Expected 54 rows");
    assert_eq!(map.tiles[0].len(), 112, "Expected 112 columns");

    // Spot-check: tile at (0,0) should have a valid type_id
    let t = map.tiles[0][0];
    eprintln!("Tile (0,0): type_id={}, index={}", t.type_id, t.index);

    // Count unique tile types
    let mut types = std::collections::HashSet::new();
    for row in &map.tiles {
        for tile in row {
            types.insert(tile.type_id);
        }
    }
    eprintln!("Unique tile types: {}", types.len());
    assert!(types.len() > 1, "Expected multiple tile types");
}
