//! Game world state — actors, players, RNG.
//!
//! This module builds the world from map data and replay metadata,
//! then computes per-tick SyncHash to verify determinism against
//! the hashes recorded in .orarep files.

use crate::math::{CPos, WPos};
use crate::rng::MersenneTwister;
use crate::sync;

/// The game world state.
pub struct World {
    /// All actors in creation order (ActorID order).
    /// Includes world actor (ID=0), player actors, and map actors.
    all_actor_ids: Vec<u32>,
    /// Actors with ISync traits, in ActorID order.
    sync_actors: Vec<sync::ActorSync>,
    /// Synced effects (projectiles etc.) — empty at tick 0.
    synced_effects: Vec<i32>,
    /// The shared RNG.
    pub rng: MersenneTwister,
    /// Player actor IDs with UnlockedRenderPlayer = true.
    unlocked_render_player_ids: Vec<u32>,
}

impl World {
    /// Compute World.SyncHash() matching the C# algorithm exactly.
    pub fn sync_hash(&self) -> i32 {
        sync::compute_world_sync_hash(
            &self.all_actor_ids,
            &self.sync_actors,
            &self.synced_effects,
            self.rng.last,
            &self.unlocked_render_player_ids,
        )
    }

    /// Compute SyncHash components separately for debugging.
    pub fn sync_hash_debug(&self) -> (i32, i32, i32) {
        // Component 1: Actor identity hashes only
        let identity = sync::compute_world_sync_hash(
            &self.all_actor_ids, &[], &[], 0, &[],
        );
        // Component 2: Trait hashes only (compute full minus identity minus rng)
        let full_no_rng = sync::compute_world_sync_hash(
            &self.all_actor_ids, &self.sync_actors, &[], 0, &[],
        );
        let traits = full_no_rng.wrapping_sub(identity);
        // Component 3: RNG last
        let rng_last = self.rng.last;
        (identity, traits, rng_last)
    }
}

/// Convert a cell position to world position (rectangular grid).
/// CenterOfCell(cell) = WPos(1024*x + 512, 1024*y + 512, 0)
pub fn center_of_cell(x: i32, y: i32) -> WPos {
    WPos::new(1024 * x + 512, 1024 * y + 512, 0)
}

/// Trait sync info for a single ISync trait.
/// Each [VerifySync] field is hashed and XOR'd together.
fn building_sync_hash(top_left: CPos) -> i32 {
    // Building has [VerifySync] CPos TopLeft only
    sync::hash_cpos(top_left)
}

fn immobile_sync_hash(top_left: CPos, center_pos: WPos) -> i32 {
    // Immobile has [VerifySync] CPos TopLeft AND WPos CenterPosition
    sync::hash_cpos(top_left) ^ sync::hash_wpos(center_pos)
}

fn health_sync_hash(hp: i32) -> i32 {
    // Health has [VerifySync] int HP
    hp
}

/// Build a World from parsed map data and game seed.
///
/// This is the initial world state (tick 0). Actor IDs are assigned:
/// 0 = World actor, 1-N = player actors, N+1.. = map actors.
pub fn build_world(
    map: &openra_data::oramap::OraMap,
    random_seed: i32,
) -> World {
    let mut rng = MersenneTwister::new(random_seed);
    let mut all_actor_ids: Vec<u32> = Vec::new();
    let mut sync_actors: Vec<sync::ActorSync> = Vec::new();
    let mut next_id: u32 = 0;

    // === Actor ID 0: World actor (no ISync traits) ===
    all_actor_ids.push(next_id);
    next_id += 1;

    // === Player actors ===
    // Order: non-playable first (in map YAML order), then playable (slot order),
    // then "Everyone" spectator.

    // Separate non-playable and playable
    let non_playable: Vec<_> = map.players.iter()
        .filter(|p| !p.playable)
        .collect();
    let playable: Vec<_> = map.players.iter()
        .filter(|p| p.playable)
        .collect();

    let mut player_actor_ids: Vec<u32> = Vec::new();

    for _p in &non_playable {
        let id = next_id;
        all_actor_ids.push(id);
        player_actor_ids.push(id);
        next_id += 1;
    }

    for _p in &playable {
        let id = next_id;
        all_actor_ids.push(id);
        player_actor_ids.push(id);
        next_id += 1;
    }

    // "Everyone" spectator player — only created when AllowSpectators is true
    // TODO: make this conditional on game settings
    let everyone_id = next_id;
    all_actor_ids.push(everyone_id);
    player_actor_ids.push(everyone_id);
    next_id += 1;

    // Player actor sync traits — each has ~13 ISync traits:
    // 6x ProductionQueue, PlayerExperience, FrozenActorLayer,
    // GpsWatcher, Shroud, PowerManager, MissionObjectives, DeveloperMode
    // TODO: Compute exact trait hashes per player.
    // At tick 0, most values are at defaults (0, false, etc.)

    // === Map actors ===
    for actor in &map.actors {
        let id = next_id;
        all_actor_ids.push(id);
        next_id += 1;

        // Compute sync trait hashes based on actor type.
        // ISync trait order follows YAML definition order:
        //   BodyOrientation (from ^SpriteActor), Building/Immobile, Health
        let mut trait_hashes = Vec::new();

        let is_tree = actor.actor_type.starts_with('t')
            && (actor.actor_type.len() == 3 || actor.actor_type.starts_with("tc"));
        let is_mine = actor.actor_type == "mine";
        let is_spawn = actor.actor_type == "mpspawn";

        let top_left = CPos::new(actor.location.0, actor.location.1);

        if is_tree {
            // Trees: BodyOrientation(1) + Building(TopLeft) + Health(HP=50000)
            trait_hashes.push(1); // BodyOrientation: QuantizedFacings
            trait_hashes.push(building_sync_hash(top_left));
            trait_hashes.push(health_sync_hash(50000));
        } else if is_mine {
            // Ore mines: BodyOrientation(1) + Building(TopLeft) — NO Health
            trait_hashes.push(1); // BodyOrientation: QuantizedFacings
            trait_hashes.push(building_sync_hash(top_left));
        } else if is_spawn {
            // mpspawn: BodyOrientation(1) + Immobile(TopLeft, CenterPosition)
            trait_hashes.push(1); // BodyOrientation: QuantizedFacings
            let center = center_of_cell(actor.location.0, actor.location.1);
            trait_hashes.push(immobile_sync_hash(top_left, center));
        }

        if !trait_hashes.is_empty() {
            sync_actors.push(sync::ActorSync {
                actor_id: id,
                trait_hashes,
            });
        }
    }

    World {
        all_actor_ids,
        sync_actors,
        synced_effects: Vec::new(),
        rng,
        unlocked_render_player_ids: Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn center_of_cell_values() {
        let pos = center_of_cell(0, 0);
        assert_eq!(pos, WPos::new(512, 512, 0));

        let pos = center_of_cell(10, 20);
        assert_eq!(pos, WPos::new(10 * 1024 + 512, 20 * 1024 + 512, 0));
    }

    #[test]
    fn building_hash_matches_cpos_bits() {
        let top_left = CPos::new(5, 10);
        assert_eq!(building_sync_hash(top_left), top_left.bits);
    }

    #[test]
    fn immobile_hash_xors_topleft_and_center() {
        let top_left = CPos::new(5, 10);
        let center = center_of_cell(5, 10);
        let hash = immobile_sync_hash(top_left, center);
        assert_eq!(hash, top_left.bits ^ center.sync_hash());
    }
}
