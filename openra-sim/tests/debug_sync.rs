//! Debug test: dump detailed SyncHash components to find the mismatch.

use openra_data::{oramap, orarep};
use openra_sim::world::{self, GameOrder, LobbyInfo, SlotInfo};

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
fn debug_orders_and_hashes() {
    let replay_data = std::fs::read(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/replays/ra-2026-02-20T001259Z.orarep")
    ).unwrap();
    let replay = orarep::parse(&replay_data).unwrap();

    let skip = ["SyncInfo", "SyncLobbyClients", "SyncLobbySlots",
                "HandshakeResponse", "HandshakeRequest", "SyncConnectionQuality",
                "FluentMessage", "StartGame"];

    // Run simulation and compare all frames
    let map_data = std::fs::read(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/maps/singles.oramap")
    ).unwrap();
    let map = oramap::parse(&map_data).unwrap();
    let settings = replay.lobby_settings().unwrap();
    let lobby = lobby_from_replay(&replay);
    let mut w = world::build_world(&map, settings.random_seed, &lobby);

    // Dump ALL orders around the mismatch area
    eprintln!("=== ALL orders for frames 30-50 ===");
    for (frame, order) in &replay.orders {
        if *frame >= 30 && *frame <= 50 {
            eprintln!("  frame={} order='{}' subject={:?} target={:?} extra={:?}",
                frame, order.order_string, order.subject_id, order.target_string, order.extra_data);
        }
    }

    let max_frame = 75;
    let mut mismatches = 0;
    let mut first_mismatch = None;

    for sh in &replay.sync_hashes {
        if sh.frame > max_frame { break; }

        let orders: Vec<GameOrder> = replay.orders.iter()
            .filter(|(f, o)| *f == sh.frame && !skip.contains(&o.order_string.as_str()))
            .map(|(_, o)| GameOrder {
                order_string: o.order_string.clone(),
                subject_id: o.subject_id,
                target_string: o.target_string.clone(),
                extra_data: o.extra_data,
            })
            .collect();

        let computed = w.process_frame(&orders);
        if sh.frame >= 16 {
            let ok = computed == sh.sync_hash;
            if !ok {
                let delta = computed.wrapping_sub(sh.sync_hash);
                eprintln!("  frame={} computed={} expected={} delta={} MISMATCH",
                    sh.frame, computed, sh.sync_hash, delta as i32);
                mismatches += 1;
                if first_mismatch.is_none() {
                    first_mismatch = Some(sh.frame);
                    eprintln!("\n=== ANALYSIS frame {} ===", sh.frame);
                    eprintln!("  delta={} (0x{:08x})", delta as i32, delta as u32);
                    eprintln!("  our rng.last={} count={}", w.rng.last, w.rng.total_count);
                    let needed = w.rng.last.wrapping_sub(delta as i32);
                    eprintln!("  needed rng.last={}", needed);
                    let mut test_rng = w.rng.clone();
                    for n in 1..=10000 {
                        test_rng.next();
                        if test_rng.last == needed {
                            eprintln!("  FOUND: {} extra RNG calls would produce needed rng.last!", n);
                            break;
                        }
                    }
                    // Check hash_player sums
                    let hp = |id: u32| openra_sim::sync::hash_player(id);
                    eprintln!("  hash_player sums: p3+p4={} p3+p4+p5={} all5={}",
                        hp(3).wrapping_add(hp(4)),
                        hp(3).wrapping_add(hp(4)).wrapping_add(hp(5)),
                        hp(1).wrapping_add(hp(2)).wrapping_add(hp(3)).wrapping_add(hp(4)).wrapping_add(hp(5)));
                }
            }
        }
    }

    if mismatches > 0 {
        panic!("{} frame mismatches (first at frame {})", mismatches, first_mismatch.unwrap());
    } else {
        eprintln!("All frames 16-{} match!", max_frame);
    }
}
