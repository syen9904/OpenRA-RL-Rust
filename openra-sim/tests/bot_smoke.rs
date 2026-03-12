//! Smoke test: verify that Bot AI can drive a game to completion.
//!
//! This test does NOT verify SyncHash (no oracle for bot-generated orders).
//! It checks that the World + Bot combination can:
//! 1. Run without panicking
//! 2. Deploy MCV, build buildings, produce units
//! 3. Attack the enemy and reach a winner

use openra_data::oramap;
use openra_sim::ai::Bot;
use openra_sim::world::{self, LobbyInfo, SlotInfo};

fn setup_world() -> world::World {
    let map_data = std::fs::read(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../tests/maps/singles.oramap")
    ).expect("Failed to read map");
    let map = oramap::parse(&map_data).expect("Failed to parse map");

    let lobby = LobbyInfo {
        starting_cash: 5000,
        allow_spectators: true,
        occupied_slots: vec![
            SlotInfo { player_reference: "Multi0".into(), faction: "england".into() },
            SlotInfo { player_reference: "Multi1".into(), faction: "ukraine".into() },
        ],
    };

    world::build_world(&map, 42, &lobby)
}

fn find_mcv_owners(world: &world::World) -> Vec<u32> {
    let snapshot = world.snapshot();
    snapshot.actors.iter()
        .filter(|a| a.actor_type == "mcv")
        .map(|a| a.owner)
        .collect()
}

#[test]
fn bot_vs_bot_plays_to_completion() {
    let mut world = setup_world();
    let mcv_owners = find_mcv_owners(&world);
    assert_eq!(mcv_owners.len(), 2, "Expected 2 MCVs for 2 players");

    let mut bot0 = Bot::new(mcv_owners[0]);
    let mut bot1 = Bot::new(mcv_owners[1]);

    let max_frames = 6000; // Both bots attack around frame 2000-2500, need time for combat
    let mut winner = None;

    for frame in 0..max_frames {
        let orders0 = bot0.tick(&world);
        let orders1 = bot1.tick(&world);

        // Drain completed buildings after bots pick them up
        world.drain_completed_buildings(mcv_owners[0]);
        world.drain_completed_buildings(mcv_owners[1]);

        let mut all_orders = orders0;
        all_orders.extend(orders1);

        let _hash = world.process_frame(&all_orders);

        // Check for winner
        if let Some(w) = world.check_winner() {
            winner = Some(w);
            eprintln!("GAME OVER at frame {}: winner = player {}", frame, w);
            break;
        }

        // Periodic status
        if frame % 500 == 0 {
            let snap = world.snapshot();
            let p0_cash = world.player_cash(mcv_owners[0]);
            let p1_cash = world.player_cash(mcv_owners[1]);
            let p0_buildings = world.player_building_types(mcv_owners[0]);
            let p1_buildings = world.player_building_types(mcv_owners[1]);
            let p0_units: usize = snap.actors.iter()
                .filter(|a| a.owner == mcv_owners[0] && a.activity != "")
                .count();
            let p1_units: usize = snap.actors.iter()
                .filter(|a| a.owner == mcv_owners[1] && a.activity != "")
                .count();
            eprintln!(
                "Frame {}: tick={} | P0: ${} buildings={:?} units={} | P1: ${} buildings={:?} units={}",
                frame, snap.tick, p0_cash, p0_buildings, p0_units, p1_cash, p1_buildings, p1_units
            );
        }
    }

    // The game should have ended
    assert!(winner.is_some(), "Game did not finish within {} frames", max_frames);
    eprintln!("Winner: player {}", winner.unwrap());
}

#[test]
fn bot_vs_idle_eliminates_opponent() {
    // One bot plays, the other does nothing. Bot should eventually destroy
    // the idle player's MCV and win.
    let mut world = setup_world();
    let mcv_owners = find_mcv_owners(&world);

    let mut bot = Bot::new(mcv_owners[0]);
    // Player 1 is idle (no bot)

    let max_frames = 3000;
    let mut winner = None;

    for frame in 0..max_frames {
        let orders = bot.tick(&world);
        world.drain_completed_buildings(mcv_owners[0]);
        let _hash = world.process_frame(&orders);

        if let Some(w) = world.check_winner() {
            winner = Some(w);
            eprintln!("Bot wins at frame {}", frame);
            break;
        }
    }

    assert!(winner.is_some(), "Bot should have eliminated idle opponent");
    assert_eq!(winner.unwrap(), mcv_owners[0], "Active bot should win");
}
