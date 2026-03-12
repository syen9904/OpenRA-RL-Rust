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

    eprintln!("=== All non-setup orders for frames 1-25 ===");
    for (frame, order) in &replay.orders {
        if *frame >= 1 && *frame <= 25 && !skip.contains(&order.order_string.as_str()) {
            eprintln!("  frame={} order='{}' subject={:?} target={:?} extra={:?}",
                frame, order.order_string, order.subject_id, order.target_string, order.extra_data);
        }
    }

    eprintln!("\n=== Expected sync hashes for frames 1-25 ===");
    for sh in &replay.sync_hashes {
        if sh.frame >= 1 && sh.frame <= 25 {
            eprintln!("  frame={} hash={}", sh.frame, sh.sync_hash);
        }
    }

    // Run simulation and show per-frame computed vs expected
    let map_data = std::fs::read(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/maps/singles.oramap")
    ).unwrap();
    let map = oramap::parse(&map_data).unwrap();
    let settings = replay.lobby_settings().unwrap();
    let lobby = lobby_from_replay(&replay);
    let mut w = world::build_world(&map, settings.random_seed, &lobby);

    eprintln!("\n=== Per-frame simulation (frames 16-25) ===");
    for sh in &replay.sync_hashes {
        if sh.frame > 25 { break; }

        let orders: Vec<GameOrder> = replay.orders.iter()
            .filter(|(f, o)| *f == sh.frame && !skip.contains(&o.order_string.as_str()))
            .map(|(_, o)| GameOrder {
                order_string: o.order_string.clone(),
                subject_id: o.subject_id,
                target_string: o.target_string.clone(),
                extra_data: o.extra_data,
            })
            .collect();

        if sh.frame == 1 {
            eprintln!("\n=== DETAILED DUMP BEFORE frame 1 (tick 0) ===");
            w.dump_sync_details();
        }
        if sh.frame == 20 {
            eprintln!("\n=== DETAILED DUMP BEFORE frame 20 (last matching frame) ===");
            w.dump_sync_details();
        }
        if sh.frame == 21 {
            eprintln!("\n=== DETAILED DUMP BEFORE frame 21 (first mismatching frame) ===");
            w.dump_sync_details();
        }
        let computed = w.process_frame(&orders);
        if sh.frame >= 16 {
            let delta = computed.wrapping_sub(sh.sync_hash);
            eprintln!("  frame={} computed={} expected={} delta={} match={}",
                sh.frame, computed, sh.sync_hash, delta as i32, computed == sh.sync_hash);
        }

        // At frame 21, brute-force find how many extra RNG calls are needed
        if sh.frame == 21 && computed != sh.sync_hash {
            eprintln!("\n=== BRUTE FORCE: finding extra RNG calls needed ===");
            eprintln!("  Current rng.last={} rng.total_count={}", w.rng.last, w.rng.total_count);
            // We need rng.last such that AFTER_TRAITS + rng.last = expected
            let after_traits = computed.wrapping_sub(w.rng.last);
            let needed_rng: i32 = sh.sync_hash.wrapping_sub(after_traits);
            eprintln!("  AFTER_TRAITS={} needed_rng_last={}", after_traits, needed_rng);

            // Clone the RNG state from BEFORE the deploy tick (frame 20's tick)
            // We need to go back... actually the RNG was already advanced.
            // Let's just try advancing from current state
            let mut test_rng = w.rng.clone();
            // Rewind: we can't rewind MT, but we can try from current state
            // Actually, let's recompute: the RNG state at frame 20's SyncHash is what we need
            // But we've already advanced past it. Let me just check forward calls.
            // Try different numbers of extra RNG calls and check what trait delta they imply
            eprintln!("  Trying extra RNG calls (0-30):");
            let mut test_rng = w.rng.clone();
            let our_rng = w.rng.last;
            for n in 0..=30 {
                let rng_last = test_rng.last;
                let rng_delta = rng_last.wrapping_sub(our_rng);
                let trait_delta = (-1514386i32).wrapping_sub(rng_delta);
                // Check if trait_delta factors by 122 (1+FACT_id=121)
                let div122 = if trait_delta != 0 && trait_delta % 122 == 0 {
                    format!("= 122 * {}", trait_delta / 122)
                } else {
                    String::new()
                };
                eprintln!("    N={}: rng_last={} rng_delta={} trait_delta={} {}",
                    n, rng_last, rng_delta, trait_delta, div122);
                test_rng.next();
            }
        }
    }
}
