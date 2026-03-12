//! Integration test: verify World.SyncHash() against values from a real replay.
//!
//! Loads the test replay + map, builds the initial world state, and checks
//! that our computed SyncHash matches the replay's recorded value.

use openra_data::{oramap, orarep};
use openra_sim::world::{self, GameOrder, LobbyInfo, SlotInfo};

/// Build LobbyInfo from parsed replay lobby settings.
fn lobby_from_replay(replay: &orarep::Replay) -> LobbyInfo {
    let settings = replay.lobby_settings().expect("No lobby settings in replay");

    let occupied_slots = settings.occupied_slots.iter().map(|(_, player_ref, faction)| {
        SlotInfo {
            player_reference: player_ref.clone(),
            faction: faction.clone(),
        }
    }).collect();

    LobbyInfo {
        starting_cash: settings.starting_cash,
        allow_spectators: settings.allow_spectators,
        occupied_slots,
    }
}

#[test]
fn sync_hash_tick0_matches_replay() {
    // Parse replay
    let replay_data = std::fs::read(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/replays/ra-2026-02-20T001259Z.orarep")
    ).expect("Failed to read replay");
    let replay = orarep::parse(&replay_data).expect("Failed to parse replay");

    let settings = replay.lobby_settings().expect("No lobby settings");
    eprintln!("RandomSeed: {}", settings.random_seed);
    eprintln!("StartingCash: {}", settings.starting_cash);
    eprintln!("AllowSpectators: {}", settings.allow_spectators);
    eprintln!("Occupied slots: {:?}", settings.occupied_slots);
    assert_eq!(settings.random_seed, -852810065);

    // Parse map
    let map_data = std::fs::read(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/maps/singles.oramap")
    ).expect("Failed to read map");
    let map = oramap::parse(&map_data).expect("Failed to parse map");

    eprintln!("Map: {} ({}x{})", map.title, map.map_size.0, map.map_size.1);
    eprintln!("Players: {}, Actors: {}", map.players.len(), map.actors.len());
    for p in &map.players {
        eprintln!("  {} (playable={}, owns_world={}, faction={})",
            p.name, p.playable, p.owns_world, p.faction);
    }

    // Build lobby info from replay
    let lobby = lobby_from_replay(&replay);

    // Build world
    let w = world::build_world(&map, settings.random_seed, &lobby);

    // Show first few sync hash entries with their frame numbers
    for (i, sh) in replay.sync_hashes.iter().take(5).enumerate() {
        eprintln!("SyncHash[{}]: frame={} hash={}", i, sh.frame, sh.sync_hash);
    }

    // Expected SyncHash from replay tick 1 (constant for early ticks before game events)
    let expected = replay.sync_hashes[0].sync_hash;
    eprintln!("\nExpected SyncHash: {}", expected);
    assert_eq!(expected, 605399687);

    w.dump_sync_details();
    let dbg = w.sync_hash_debug();
    eprintln!("Computed SyncHash: {}", dbg.full);
    eprintln!("  Identity hashes: {}", dbg.identity);
    eprintln!("  Trait hashes: {}", dbg.traits);
    eprintln!("  RNG last: {}", dbg.rng_last);

    if dbg.full != expected {
        eprintln!("\nMISMATCH: computed={} expected={} diff={}",
            dbg.full, expected, dbg.full.wrapping_sub(expected));
    }

    assert_eq!(dbg.full, expected, "SyncHash mismatch at tick 0");
}

/// Convert replay orders to GameOrder structs for a given frame.
fn orders_for_frame(replay: &orarep::Replay, frame: i32) -> Vec<GameOrder> {
    // Skip lobby/setup orders that don't affect simulation
    let skip = ["SyncInfo", "SyncLobbyClients", "SyncLobbySlots",
                "HandshakeResponse", "HandshakeRequest", "SyncConnectionQuality",
                "FluentMessage", "StartGame", "Command"];

    replay.orders.iter()
        .filter(|(f, o)| *f == frame && !skip.contains(&o.order_string.as_str()))
        .map(|(_, o)| GameOrder {
            order_string: o.order_string.clone(),
            subject_id: o.subject_id,
            target_string: o.target_string.clone(),
            extra_data: o.extra_data,
        })
        .collect()
}

#[test]
fn sync_hash_multi_frame() {
    let replay_data = std::fs::read(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/replays/ra-2026-02-20T001259Z.orarep")
    ).expect("Failed to read replay");
    let replay = orarep::parse(&replay_data).expect("Failed to parse replay");

    let map_data = std::fs::read(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/maps/singles.oramap")
    ).expect("Failed to read map");
    let map = oramap::parse(&map_data).expect("Failed to parse map");

    let settings = replay.lobby_settings().expect("No lobby settings");
    let lobby = lobby_from_replay(&replay);
    let mut w = world::build_world(&map, settings.random_seed, &lobby);

    let mut matched = 0;
    let mut total = replay.sync_hashes.len();

    for sh in &replay.sync_hashes {
        let orders = orders_for_frame(&replay, sh.frame);
        let computed = w.process_frame(&orders);

        if computed == sh.sync_hash {
            matched += 1;
        } else {
            eprintln!("MISMATCH frame={} computed={} expected={} delta={}",
                sh.frame, computed, sh.sync_hash,
                computed.wrapping_sub(sh.sync_hash));
            // Stop at first mismatch for debugging
            break;
        }
    }

    eprintln!("\nMatched {}/{} frames", matched, total);
    assert!(matched >= 16, "Should match at least frames 1-16, got {}", matched);
}
