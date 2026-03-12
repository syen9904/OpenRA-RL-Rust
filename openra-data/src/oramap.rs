//! `.oramap` map file parser.
//!
//! An .oramap file is a ZIP archive containing:
//! - `map.yaml` — map metadata, player definitions, actor list
//! - `map.bin` — terrain tile data
//! - `map.png` — preview image (ignored)
//!
//! For the MVP we only parse map.yaml to get:
//! - Map dimensions and bounds
//! - Player definitions
//! - Initial actors (trees, mines, spawn points)

use std::io::{self, Read};

/// A player definition from the map
#[derive(Debug, Clone)]
pub struct PlayerDef {
    pub name: String,
    pub playable: bool,
    pub owns_world: bool,
    pub non_combatant: bool,
    pub faction: String,
    pub enemies: Vec<String>,
}

/// An actor placed on the map
#[derive(Debug, Clone)]
pub struct MapActor {
    pub id: String,
    pub actor_type: String,
    pub owner: String,
    pub location: (i32, i32),
}

/// A terrain tile reference (type + index within tileset).
#[derive(Debug, Clone, Copy, Default)]
pub struct TileReference {
    /// Terrain type ID from the tileset.
    pub type_id: u16,
    /// Variant index within the type.
    pub index: u8,
}

/// Parsed map data
#[derive(Debug, Clone)]
pub struct OraMap {
    pub title: String,
    pub tileset: String,
    pub map_size: (i32, i32),
    pub bounds: (i32, i32, i32, i32), // x, y, w, h
    pub players: Vec<PlayerDef>,
    pub actors: Vec<MapActor>,
    /// Terrain tile grid [row][col], dimensions = map_size.
    /// Empty if map.bin was not found or could not be parsed.
    pub tiles: Vec<Vec<TileReference>>,
}

/// Parse a simple MiniYaml value from a line like "Key: Value"
fn parse_kv(line: &str) -> Option<(&str, &str)> {
    let line = line.trim();
    let colon = line.find(':')?;
    let key = line[..colon].trim();
    let value = line[colon + 1..].trim();
    Some((key, value))
}

/// Parse "X,Y" into (i32, i32)
fn parse_pair(s: &str) -> Option<(i32, i32)> {
    let mut parts = s.split(',');
    let x = parts.next()?.trim().parse().ok()?;
    let y = parts.next()?.trim().parse().ok()?;
    Some((x, y))
}

/// Parse "X,Y,W,H" into (i32, i32, i32, i32)
fn parse_quad(s: &str) -> Option<(i32, i32, i32, i32)> {
    let mut parts = s.split(',');
    let a = parts.next()?.trim().parse().ok()?;
    let b = parts.next()?.trim().parse().ok()?;
    let c = parts.next()?.trim().parse().ok()?;
    let d = parts.next()?.trim().parse().ok()?;
    Some((a, b, c, d))
}

/// Parse map.yaml content into an OraMap struct.
pub fn parse_map_yaml(yaml: &str) -> io::Result<OraMap> {
    let mut title = String::new();
    let mut tileset = String::new();
    let mut map_size = (0, 0);
    let mut bounds = (0, 0, 0, 0);
    let mut players = Vec::new();
    let mut actors = Vec::new();

    let lines: Vec<&str> = yaml.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let line = lines[i];
        let trimmed = line.trim();

        if trimmed.is_empty() || trimmed.starts_with('#') {
            i += 1;
            continue;
        }

        if let Some((key, value)) = parse_kv(trimmed) {
            match key {
                "Title" => title = value.to_string(),
                "Tileset" => tileset = value.to_string(),
                "MapSize" => {
                    if let Some(p) = parse_pair(value) {
                        map_size = p;
                    }
                }
                "Bounds" => {
                    if let Some(q) = parse_quad(value) {
                        bounds = q;
                    }
                }
                _ => {}
            }
        }

        // Parse Players section
        if trimmed == "Players:" {
            i += 1;
            while i < lines.len() {
                let pline = lines[i];
                // Check if we're still in Players section (indented with tab)
                if !pline.starts_with('\t') && !pline.starts_with("  ") {
                    break;
                }

                let ptrimmed = pline.trim();
                // PlayerReference@Name:
                if ptrimmed.starts_with("PlayerReference@") && ptrimmed.ends_with(':') {
                    let ref_name = &ptrimmed["PlayerReference@".len()..ptrimmed.len() - 1];
                    let mut player = PlayerDef {
                        name: ref_name.to_string(),
                        playable: false,
                        owns_world: false,
                        non_combatant: false,
                        faction: String::new(),
                        enemies: Vec::new(),
                    };

                    i += 1;
                    // Parse player properties (double-indented)
                    while i < lines.len() {
                        let ppline = lines[i];
                        let depth = ppline.len() - ppline.trim_start().len();
                        if depth < 2 && !ppline.trim().is_empty() {
                            break;
                        }
                        if ppline.trim().is_empty() {
                            i += 1;
                            continue;
                        }

                        if let Some((k, v)) = parse_kv(ppline.trim()) {
                            match k {
                                "Name" => player.name = v.to_string(),
                                "Playable" => player.playable = v == "True",
                                "OwnsWorld" => player.owns_world = v == "True",
                                "NonCombatant" => player.non_combatant = v == "True",
                                "Faction" => player.faction = v.to_lowercase(),
                                "Enemies" => {
                                    player.enemies = v.split(',').map(|s| s.trim().to_string()).collect();
                                }
                                _ => {}
                            }
                        }
                        i += 1;
                    }
                    players.push(player);
                    continue;
                }
                i += 1;
            }
            continue;
        }

        // Parse Actors section
        if trimmed == "Actors:" {
            i += 1;
            while i < lines.len() {
                let aline = lines[i];
                if !aline.starts_with('\t') && !aline.starts_with("  ") {
                    break;
                }

                let atrimmed = aline.trim();
                // Actor line: "ActorXXX: type_name"
                if atrimmed.contains(':') && !atrimmed.starts_with("Owner") && !atrimmed.starts_with("Location") {
                    if let Some((actor_key, actor_type)) = parse_kv(atrimmed) {
                        let mut actor = MapActor {
                            id: actor_key.to_string(),
                            actor_type: actor_type.to_string(),
                            owner: String::new(),
                            location: (0, 0),
                        };

                        i += 1;
                        // Parse actor properties
                        while i < lines.len() {
                            let apline = lines[i];
                            let depth = apline.len() - apline.trim_start().len();
                            if depth < 2 && !apline.trim().is_empty() {
                                break;
                            }
                            if apline.trim().is_empty() {
                                i += 1;
                                continue;
                            }

                            if let Some((k, v)) = parse_kv(apline.trim()) {
                                match k {
                                    "Owner" => actor.owner = v.to_string(),
                                    "Location" => {
                                        if let Some(p) = parse_pair(v) {
                                            actor.location = p;
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            i += 1;
                        }
                        actors.push(actor);
                        continue;
                    }
                }
                i += 1;
            }
            continue;
        }

        i += 1;
    }

    Ok(OraMap {
        title,
        tileset,
        map_size,
        bounds,
        players,
        actors,
        tiles: Vec::new(),
    })
}

/// Parse terrain tiles from map.bin binary data.
///
/// Format: sequence of (u16 type_id, u8 index) packed per cell,
/// row-major order, dimensions from map_size.
///
/// Reference: OpenRA.Game/Map/Map.cs PostInit(), CellLayer<TerrainTile>
fn parse_map_bin(data: &[u8], width: i32, height: i32) -> Vec<Vec<TileReference>> {
    let w = width as usize;
    let h = height as usize;
    let mut tiles = vec![vec![TileReference::default(); w]; h];

    // Each tile is 3 bytes: u16 LE type_id + u8 index
    let bytes_per_tile = 3;
    let expected_size = w * h * bytes_per_tile;

    if data.len() < expected_size {
        return tiles;
    }

    for row in 0..h {
        for col in 0..w {
            let offset = (row * w + col) * bytes_per_tile;
            let type_id = u16::from_le_bytes([data[offset], data[offset + 1]]);
            let index = data[offset + 2];
            tiles[row][col] = TileReference { type_id, index };
        }
    }

    tiles
}

/// Parse an .oramap file (ZIP archive) from bytes.
pub fn parse(data: &[u8]) -> io::Result<OraMap> {
    let cursor = io::Cursor::new(data);
    let mut archive = zip::ZipArchive::new(cursor)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;

    let mut yaml_content = String::new();
    {
        let mut yaml_file = archive
            .by_name("map.yaml")
            .map_err(|e| io::Error::new(io::ErrorKind::NotFound, e))?;
        yaml_file.read_to_string(&mut yaml_content)?;
    }

    let mut map = parse_map_yaml(&yaml_content)?;

    // Try to parse map.bin for terrain data
    if let Ok(mut bin_file) = archive.by_name("map.bin") {
        let mut bin_data = Vec::new();
        if bin_file.read_to_end(&mut bin_data).is_ok() {
            map.tiles = parse_map_bin(&bin_data, map.map_size.0, map.map_size.1);
        }
    }

    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_yaml() {
        let yaml = r#"
Title: Test Map
Tileset: TEMPERAT
MapSize: 64,64
Bounds: 2,2,60,60

Players:
	PlayerReference@Neutral:
		Name: Neutral
		OwnsWorld: True
		NonCombatant: True
		Faction: allies
	PlayerReference@Multi0:
		Name: Multi0
		Playable: True
		Faction: Random

Actors:
	Actor0: mpspawn
		Owner: Neutral
		Location: 10,10
	Actor1: mine
		Owner: Neutral
		Location: 20,20
"#;
        let map = parse_map_yaml(yaml).unwrap();
        assert_eq!(map.title, "Test Map");
        assert_eq!(map.tileset, "TEMPERAT");
        assert_eq!(map.map_size, (64, 64));
        assert_eq!(map.bounds, (2, 2, 60, 60));
        assert_eq!(map.players.len(), 2);
        assert_eq!(map.players[0].name, "Neutral");
        assert!(map.players[0].owns_world);
        assert!(map.players[1].playable);
        assert_eq!(map.actors.len(), 2);
        assert_eq!(map.actors[0].actor_type, "mpspawn");
        assert_eq!(map.actors[0].location, (10, 10));
        assert_eq!(map.actors[1].actor_type, "mine");
        assert_eq!(map.actors[1].location, (20, 20));
    }
}
