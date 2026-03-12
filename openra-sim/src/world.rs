//! Game world state — actors, players, RNG.
//!
//! This module builds the world from map data and replay metadata,
//! then computes per-tick SyncHash to verify determinism against
//! the hashes recorded in .orarep files.

use std::collections::{BTreeMap, HashMap};

use serde::Serialize;

pub use crate::actor::ActorKind;

use crate::actor::{Activity, Actor};
use crate::math::{CPos, WPos};
use crate::pathfinder;
use crate::rng::MersenneTwister;
use crate::sync;
use crate::terrain::TerrainMap;
use crate::traits::{PqType, TraitState};

/// Lobby information extracted from the replay's SyncInfo orders.
#[derive(Debug, Clone)]
pub struct LobbyInfo {
    pub starting_cash: i32,
    pub allow_spectators: bool,
    pub occupied_slots: Vec<SlotInfo>,
}

#[derive(Debug, Clone)]
pub struct SlotInfo {
    pub player_reference: String,
    pub faction: String,
}

impl Default for LobbyInfo {
    fn default() -> Self {
        LobbyInfo {
            starting_cash: 5000,
            allow_spectators: true,
            occupied_slots: Vec::new(),
        }
    }
}

/// A game order parsed from the replay, ready for dispatch.
#[derive(Debug, Clone)]
pub struct GameOrder {
    pub order_string: String,
    pub subject_id: Option<u32>,
    pub target_string: Option<String>,
    pub extra_data: Option<u32>,
}

/// A production item being built in a ClassicProductionQueue.
#[derive(Debug)]
struct ProductionItem {
    item_name: String,
    total_cost: i32,
    total_time: i32,
    remaining_cost: i32,
    remaining_time: i32,
    started: bool,
}

impl ProductionItem {
    fn new(name: &str, cost: i32, build_duration_modifier: i32) -> Self {
        let time = (cost as i64 * build_duration_modifier as i64 / 100) as i32;
        ProductionItem {
            item_name: name.to_string(),
            total_cost: cost,
            total_time: time,
            remaining_cost: cost,
            remaining_time: time,
            started: false,
        }
    }

    fn tick(&mut self, cash: i32) -> i32 {
        if !self.started {
            self.started = true;
        }
        if self.remaining_time <= 0 {
            return 0;
        }
        if self.remaining_cost != 0 {
            let expected_remaining_cost = if self.remaining_time == 1 {
                0
            } else {
                (self.total_cost as i64 * self.remaining_time as i64
                    / self.total_time.max(1) as i64) as i32
            };
            let cost_this_tick = self.remaining_cost - expected_remaining_cost;
            if cost_this_tick != 0 {
                if cash < cost_this_tick {
                    return 0;
                }
                self.remaining_cost -= cost_this_tick;
                self.remaining_time -= 1;
                return cost_this_tick;
            }
        }
        self.remaining_time -= 1;
        0
    }

    fn is_done(&self) -> bool {
        self.remaining_time <= 0
    }
}

/// A deferred action to execute at the end of World.Tick().
#[derive(Debug)]
enum FrameEndTask {
    DeployTransform { old_actor_id: u32, location: (i32, i32), owner_player_id: u32 },
    SpawnUnit { unit_type: String, owner_player_id: u32 },
}

// === Snapshot types for rendering ===

#[derive(Debug, Serialize)]
pub struct WorldSnapshot {
    pub tick: u32,
    pub actors: Vec<ActorSnapshot>,
    pub players: Vec<PlayerSnapshot>,
    pub map_width: i32,
    pub map_height: i32,
}

#[derive(Debug, Serialize)]
pub struct ActorSnapshot {
    pub id: u32,
    pub kind: ActorKind,
    pub owner: u32,
    pub x: i32,
    pub y: i32,
}

#[derive(Debug, Serialize)]
pub struct PlayerSnapshot {
    pub index: u32,
    pub cash: i32,
}

pub struct SyncHashDebug {
    pub full: i32,
    pub identity: i32,
    pub traits: i32,
    pub rng_last: i32,
}

/// The game world state.
pub struct World {
    /// All actors keyed by ID. BTreeMap ensures deterministic iteration order.
    actors: BTreeMap<u32, Actor>,
    /// Synced effects (projectiles etc.) — empty for now.
    synced_effects: Vec<i32>,
    /// The shared RNG.
    pub rng: MersenneTwister,
    /// Whether the world simulation is paused.
    pub paused: bool,
    /// Current simulation tick.
    pub world_tick: u32,
    /// Current frame number.
    pub frame_number: u32,
    /// Network order latency (empty frames before game starts).
    pub order_latency: u32,
    /// Next actor ID to assign.
    next_actor_id: u32,
    /// Pending frame-end tasks.
    frame_end_tasks: Vec<FrameEndTask>,
    /// Player actor IDs in creation order.
    player_actor_ids: Vec<u32>,
    /// "Everyone" spectator player ID.
    everyone_player_id: u32,
    /// Number of mine actors (for SeedsResource RNG consumption).
    mine_count: usize,
    /// Ticks until next SeedsResource seeding event.
    seeds_resource_ticks: i32,
    /// Active production items per player actor ID.
    production: HashMap<u32, Vec<ProductionItem>>,
    /// Terrain and occupancy map.
    pub terrain: TerrainMap,
    /// Map dimensions (cells).
    map_width: i32,
    map_height: i32,
}

impl World {
    /// Compute World.SyncHash() matching the C# algorithm exactly.
    pub fn sync_hash(&self) -> i32 {
        let actor_ids: Vec<u32> = self.actors.keys().copied().collect();
        let actor_syncs: Vec<sync::ActorSync> = self.actors.values()
            .filter(|a| !a.traits.is_empty())
            .map(|a| sync::ActorSync {
                actor_id: a.id,
                trait_hashes: a.sync_hashes(),
            })
            .collect();
        sync::compute_world_sync_hash(
            &actor_ids,
            &actor_syncs,
            &self.synced_effects,
            self.rng.last,
            &[], // unlocked_render_player_ids — always empty for now
        )
    }

    /// Compute SyncHash components separately for debugging.
    pub fn sync_hash_debug(&self) -> SyncHashDebug {
        let actor_ids: Vec<u32> = self.actors.keys().copied().collect();
        let identity = sync::compute_world_sync_hash(&actor_ids, &[], &[], 0, &[]);
        let actor_syncs: Vec<sync::ActorSync> = self.actors.values()
            .filter(|a| !a.traits.is_empty())
            .map(|a| sync::ActorSync {
                actor_id: a.id,
                trait_hashes: a.sync_hashes(),
            })
            .collect();
        let full_no_rng = sync::compute_world_sync_hash(&actor_ids, &actor_syncs, &[], 0, &[]);
        let traits = full_no_rng.wrapping_sub(identity);
        SyncHashDebug { full: self.sync_hash(), identity, traits, rng_last: self.rng.last }
    }

    /// Create a snapshot of the current world state for rendering.
    pub fn snapshot(&self) -> WorldSnapshot {
        let mut actors = Vec::new();
        for actor in self.actors.values() {
            if actor.kind == ActorKind::World || actor.kind == ActorKind::Player
                || actor.kind == ActorKind::Spawn
            {
                continue;
            }
            let owner = actor.owner_id.unwrap_or(0);
            let (x, y) = actor.location.unwrap_or((0, 0));
            actors.push(ActorSnapshot { id: actor.id, kind: actor.kind, owner, x, y });
        }
        let players = self.player_actor_ids.iter().map(|&pid| {
            let cash = self.actors.get(&pid).map(|a| a.cash()).unwrap_or(0);
            PlayerSnapshot { index: pid, cash }
        }).collect();
        WorldSnapshot {
            tick: self.world_tick,
            actors,
            players,
            map_width: self.map_width,
            map_height: self.map_height,
        }
    }

    /// Dump per-actor, per-trait contributions for debugging.
    pub fn dump_sync_details(&self) {
        let mut n: i32 = 0;
        let mut ret: i32 = 0;

        for (&actor_id, _) in &self.actors {
            let contrib = n.wrapping_mul((1i32).wrapping_add(actor_id as i32))
                .wrapping_mul(sync::hash_actor(actor_id));
            eprintln!("IDENTITY n={} actor_id={} contrib={} running={}",
                n, actor_id, contrib, ret.wrapping_add(contrib));
            ret = ret.wrapping_add(contrib);
            n += 1;
        }
        eprintln!("AFTER_IDENTITY ret={} n={}", ret, n);

        for actor in self.actors.values() {
            if actor.traits.is_empty() { continue; }
            for (ti, t) in actor.traits.iter().enumerate() {
                let trait_hash = t.sync_hash();
                let contrib = n.wrapping_mul((1i32).wrapping_add(actor.id as i32))
                    .wrapping_mul(trait_hash);
                eprintln!("TRAIT n={} actor_id={} trait_idx={} hash={} contrib={} running={}",
                    n, actor.id, ti, trait_hash, contrib, ret.wrapping_add(contrib));
                ret = ret.wrapping_add(contrib);
                n += 1;
            }
        }
        eprintln!("AFTER_TRAITS ret={} n={}", ret, n);
        eprintln!("RNG_LAST={}", self.rng.last);
        ret = ret.wrapping_add(self.rng.last);
        eprintln!("FINAL ret={}", ret);
    }

    /// Process one frame of the simulation.
    ///
    /// C# execution order per frame:
    /// 1. Auto-unpause after orderLatency buffer period
    /// 2. ProcessOrders() — resolve orders from replay
    /// 3. SyncHash() — computed (this is what the replay records)
    /// 4. World.Tick() — advance simulation if not paused
    pub fn process_frame(&mut self, orders: &[GameOrder]) -> i32 {
        self.frame_number += 1;

        // Auto-unpause after orderLatency buffer period.
        if self.paused && self.frame_number > self.order_latency {
            self.paused = false;
            self.update_debug_pause_state();
        }

        // 1. Process orders
        for order in orders {
            self.process_order(order);
        }

        // 2. Compute SyncHash
        let hash = self.sync_hash();

        // 3. Tick the world if not paused (NetFrameInterval=3)
        if !self.paused {
            for _ in 0..3 {
                self.world_tick += 1;
                self.tick_actors();
                self.execute_frame_end_tasks();
            }
        }

        hash
    }

    /// Update DebugPauseState on the World actor (ID=0).
    fn update_debug_pause_state(&mut self) {
        if let Some(world_actor) = self.actors.get_mut(&0) {
            for t in &mut world_actor.traits {
                if let TraitState::DebugPauseState { paused } = t {
                    *paused = self.paused;
                    return;
                }
            }
        }
    }

    /// Process a single game order.
    fn process_order(&mut self, order: &GameOrder) {
        match order.order_string.as_str() {
            "PauseGame" => {
                if let Some(ref ts) = order.target_string {
                    self.paused = ts == "Pause";
                }
            }
            "DeployTransform" => {
                if let Some(subject_id) = order.subject_id {
                    if let Some(actor) = self.actors.get_mut(&subject_id) {
                        if actor.kind == ActorKind::Mcv {
                            actor.activity = Some(Activity::Turn { target: 384, speed: 20 });
                            eprintln!("ORDER: DeployTransform subject={}", subject_id);
                        }
                    }
                }
            }
            "StartProduction" => {
                if let (Some(subject_id), Some(item_name)) = (order.subject_id, &order.target_string) {
                    let cost = production_cost(item_name);
                    if cost > 0 {
                        eprintln!("ORDER: StartProduction subject={} item={} cost={}", subject_id, item_name, cost);
                        let item = ProductionItem::new(item_name, cost, 60);
                        self.production.entry(subject_id).or_default().push(item);
                    }
                }
            }
            "Move" => {
                if let Some(subject_id) = order.subject_id {
                    if let Some(ref ts) = order.target_string {
                        // Parse "X,Y" target cell
                        if let Some(target) = parse_cell_target(ts) {
                            self.order_move(subject_id, target);
                        }
                    }
                }
            }
            "AttackMove" => {
                if let Some(subject_id) = order.subject_id {
                    if let Some(ref ts) = order.target_string {
                        if let Some(target) = parse_cell_target(ts) {
                            self.order_move(subject_id, target);
                        }
                    }
                }
            }
            "Attack" => {
                if let Some(subject_id) = order.subject_id {
                    if let Some(target_actor_id) = order.extra_data {
                        self.order_attack(subject_id, target_actor_id);
                    }
                }
            }
            "PlaceBuilding" => {
                if let (Some(subject_id), Some(ts)) = (order.subject_id, &order.target_string) {
                    // Format: "building_type,X,Y"
                    let parts: Vec<&str> = ts.split(',').collect();
                    if parts.len() >= 3 {
                        let building_type = parts[0].trim();
                        if let (Ok(x), Ok(y)) = (parts[1].trim().parse::<i32>(), parts[2].trim().parse::<i32>()) {
                            self.order_place_building(subject_id, building_type, x, y);
                        }
                    }
                }
            }
            "Stop" => {
                if let Some(subject_id) = order.subject_id {
                    if let Some(actor) = self.actors.get_mut(&subject_id) {
                        actor.activity = None;
                    }
                }
            }
            "StartGame" | "Command" => {}
            other => {
                eprintln!("ORDER: unhandled '{}' subject={:?}", other, order.subject_id);
            }
        }
    }

    /// Handle a Move order: pathfind and start moving.
    fn order_move(&mut self, actor_id: u32, target: (i32, i32)) {
        let from = match self.actors.get(&actor_id).and_then(|a| a.location) {
            Some(loc) => loc,
            None => return,
        };
        if let Some(path) = pathfinder::find_path(&self.terrain, from, target, Some(actor_id)) {
            if path.len() > 1 {
                // Get speed from rules or use default
                let speed = self.actor_speed(actor_id);
                if let Some(actor) = self.actors.get_mut(&actor_id) {
                    actor.activity = Some(Activity::Move {
                        path,
                        path_index: 1, // Skip the start cell
                        speed,
                    });
                }
            }
        }
    }

    /// Handle an Attack order: set attack activity.
    fn order_attack(&mut self, actor_id: u32, target_id: u32) {
        // Default weapon range of 5 cells (will be from rules later)
        if let Some(actor) = self.actors.get_mut(&actor_id) {
            actor.activity = Some(Activity::Attack {
                target_id,
                weapon_range: 5,
            });
        }
    }

    /// Handle PlaceBuilding order: create building actor and occupy terrain.
    fn order_place_building(&mut self, owner_player_id: u32, building_type: &str, x: i32, y: i32) {
        let (footprint_w, footprint_h, hp) = building_footprint(building_type);
        let building_id = self.next_actor_id;
        self.next_actor_id += 1;

        let top_left = CPos::new(x, y);
        let building = Actor {
            id: building_id,
            kind: ActorKind::Building,
            owner_id: Some(owner_player_id),
            location: Some((x, y)),
            traits: vec![
                TraitState::BodyOrientation { quantized_facings: 1 },
                TraitState::Building { top_left },
                TraitState::Health { hp },
                TraitState::RevealsShroud,
            ],
            activity: None,
        };
        self.actors.insert(building_id, building);
        self.terrain.occupy_footprint(x, y, footprint_w, footprint_h, building_id);

        eprintln!("PLACE: {} at ({},{}) id={} footprint={}x{}",
            building_type, x, y, building_id, footprint_w, footprint_h);
    }

    /// Get movement speed for an actor (world units per tick).
    fn actor_speed(&self, actor_id: u32) -> i32 {
        if let Some(actor) = self.actors.get(&actor_id) {
            match actor.kind {
                ActorKind::Infantry => 43,   // ~24 ticks/cell
                ActorKind::Vehicle => 85,    // ~12 ticks/cell
                ActorKind::Mcv => 56,        // ~18 ticks/cell
                _ => 56,
            }
        } else {
            56
        }
    }

    /// Tick all actors (activities and ITick traits).
    fn tick_actors(&mut self) {
        // On tick 1, ClassicProductionQueue.Tick() sets Enabled=false
        // (no Production buildings exist yet).
        if self.world_tick == 1 {
            for &pid in &self.player_actor_ids {
                if let Some(player) = self.actors.get_mut(&pid) {
                    for t in &mut player.traits {
                        if let TraitState::ClassicProductionQueue { enabled, .. } = t {
                            *enabled = false;
                        }
                    }
                }
            }
        }

        // SeedsResource: each MINE actor seeds ore every 75 ticks.
        // Fires at ticks 1, 76, 151, ... consuming 2 RNG calls per mine.
        if self.seeds_resource_ticks > 0 {
            self.seeds_resource_ticks -= 1;
        }
        if self.seeds_resource_ticks <= 0 {
            for _ in 0..self.mine_count {
                self.rng.next_range(-1, 2); // dx
                self.rng.next_range(-1, 2); // dy
            }
            self.seeds_resource_ticks = 75;
        }

        // Tick Turn activities: change facing toward target.
        // When facing reaches target, queue deploy for MCVs.
        let mut deploy_ready: Vec<(u32, (i32, i32), u32)> = Vec::new();
        for actor in self.actors.values_mut() {
            if let Some(Activity::Turn { target, speed }) = &actor.activity {
                let target = *target;
                let speed = *speed;

                // Read current facing from Mobile trait
                let current_facing = actor.traits.iter()
                    .find_map(|t| {
                        if let TraitState::Mobile { facing, .. } = t { Some(*facing) } else { None }
                    })
                    .unwrap_or(0);

                let new_facing = tick_facing(current_facing, target, speed);

                // Update Mobile trait
                for t in &mut actor.traits {
                    if let TraitState::Mobile { facing, .. } = t {
                        *facing = new_facing;
                        break;
                    }
                }

                if new_facing == target {
                    actor.activity = None;
                    if actor.kind == ActorKind::Mcv {
                        if let (Some(loc), Some(owner)) = (actor.location, actor.owner_id) {
                            deploy_ready.push((actor.id, loc, owner));
                        }
                    }
                }
            }
        }

        // Tick Move activities: advance position along path.
        let mut move_completions: Vec<u32> = Vec::new();
        for actor in self.actors.values_mut() {
            if let Some(Activity::Move { ref path, ref mut path_index, speed }) = actor.activity {
                if *path_index >= path.len() {
                    move_completions.push(actor.id);
                    continue;
                }
                let target_cell = path[*path_index];
                let target_center = center_of_cell(target_cell.0, target_cell.1);

                // Update facing toward target
                if let Some(current_loc) = actor.location {
                    let desired_facing = pathfinder::facing_between(current_loc, target_cell);
                    for t in &mut actor.traits {
                        if let TraitState::Mobile { facing, .. } = t {
                            *facing = desired_facing;
                            break;
                        }
                    }
                }

                // Interpolate CenterPosition toward target
                let mut arrived = false;
                for t in &mut actor.traits {
                    if let TraitState::Mobile { center_position, from_cell, to_cell, .. } = t {
                        // Set ToCell to target
                        *to_cell = CPos::new(target_cell.0, target_cell.1);

                        let dx = target_center.x - center_position.x;
                        let dy = target_center.y - center_position.y;
                        let dist_sq = (dx as i64) * (dx as i64) + (dy as i64) * (dy as i64);
                        let speed_sq = (speed as i64) * (speed as i64);

                        if dist_sq <= speed_sq {
                            // Arrived at cell
                            *center_position = target_center;
                            *from_cell = CPos::new(target_cell.0, target_cell.1);
                            *to_cell = CPos::new(target_cell.0, target_cell.1);
                            arrived = true;
                        } else {
                            // Move toward target
                            // Use integer approximation: normalize by largest component
                            let abs_dx = dx.abs().max(1);
                            let abs_dy = dy.abs().max(1);
                            let max_comp = abs_dx.max(abs_dy);
                            center_position.x += dx * speed / max_comp;
                            center_position.y += dy * speed / max_comp;
                        }
                        break;
                    }
                }

                if arrived {
                    actor.location = Some(target_cell);
                    *path_index += 1;
                    if *path_index >= path.len() {
                        move_completions.push(actor.id);
                    }
                }
            }
        }
        // Clear completed Move activities
        for id in move_completions {
            if let Some(actor) = self.actors.get_mut(&id) {
                actor.activity = None;
            }
        }

        // Tick Attack activities: check range, deal damage.
        let mut attacks: Vec<(u32, u32, i32)> = Vec::new(); // (attacker, target, damage)
        for actor in self.actors.values() {
            if let Some(Activity::Attack { target_id, weapon_range }) = &actor.activity {
                if let (Some(attacker_loc), Some(target)) =
                    (actor.location, self.actors.get(target_id))
                {
                    if let Some(target_loc) = target.location {
                        let dx = (attacker_loc.0 - target_loc.0).abs();
                        let dy = (attacker_loc.1 - target_loc.1).abs();
                        let dist = dx.max(dy);
                        if dist <= *weapon_range {
                            // In range — deal damage (simplified: 100 HP per tick)
                            attacks.push((actor.id, *target_id, 100));
                        }
                    }
                }
            }
        }
        // Apply damage
        let mut dead_actors: Vec<u32> = Vec::new();
        for (_attacker, target_id, damage) in &attacks {
            if let Some(target) = self.actors.get_mut(target_id) {
                for t in &mut target.traits {
                    if let TraitState::Health { hp } = t {
                        *hp -= damage;
                        if *hp <= 0 {
                            dead_actors.push(*target_id);
                        }
                        break;
                    }
                }
            }
        }
        // Remove dead actors
        for id in dead_actors {
            if let Some(dead) = self.actors.remove(&id) {
                if let Some(loc) = dead.location {
                    self.terrain.clear_occupant(loc.0, loc.1);
                }
            }
        }

        // Tick production queues: consume cash, advance build time.
        let player_ids: Vec<u32> = self.production.keys().copied().collect();
        let mut completed_items: Vec<(u32, String)> = Vec::new();
        for pid in player_ids {
            let cash = self.actors.get(&pid).map(|a| a.cash()).unwrap_or(0);
            if let Some(items) = self.production.get_mut(&pid) {
                if let Some(item) = items.first_mut() {
                    let consumed = item.tick(cash);
                    if consumed > 0 {
                        if let Some(player) = self.actors.get_mut(&pid) {
                            player.set_cash(cash - consumed);
                        }
                    }
                    if item.is_done() {
                        let name = item.item_name.clone();
                        items.remove(0);
                        eprintln!("PRODUCTION: {} complete for player {}", name, pid);
                        completed_items.push((pid, name));
                    }
                }
            }
        }
        // Spawn completed units
        for (owner_pid, unit_type) in completed_items {
            if is_unit_type(&unit_type) {
                self.frame_end_tasks.push(FrameEndTask::SpawnUnit {
                    unit_type,
                    owner_player_id: owner_pid,
                });
            }
            // Buildings wait for PlaceBuilding order
        }

        // Queue deploy for MCVs that finished turning
        for (actor_id, location, owner_player_id) in deploy_ready {
            self.frame_end_tasks.push(FrameEndTask::DeployTransform {
                old_actor_id: actor_id,
                location,
                owner_player_id,
            });
        }
    }

    /// Execute frame-end tasks (actor removal/creation from Transform, etc.)
    fn execute_frame_end_tasks(&mut self) {
        let tasks: Vec<_> = self.frame_end_tasks.drain(..).collect();
        for task in tasks {
            match task {
                FrameEndTask::DeployTransform { old_actor_id, location, owner_player_id } => {
                    self.deploy_transform(old_actor_id, location, owner_player_id);
                }
                FrameEndTask::SpawnUnit { unit_type, owner_player_id } => {
                    self.spawn_unit(&unit_type, owner_player_id);
                }
            }
        }
    }

    /// Spawn a unit near the owner's production building (WEAP for vehicles, TENT/BARR for infantry).
    fn spawn_unit(&mut self, unit_type: &str, owner_player_id: u32) {
        // Find a building owned by this player to spawn near
        let spawn_loc = self.find_spawn_location(owner_player_id, unit_type);
        let (x, y) = match spawn_loc {
            Some(loc) => loc,
            None => return, // No production building found
        };

        let unit_id = self.next_actor_id;
        self.next_actor_id += 1;

        let (kind, speed, hp) = unit_stats(unit_type);
        let facing = 512; // Default facing south
        let cell = CPos::new(x, y);
        let center = center_of_cell(x, y);

        let actor = Actor {
            id: unit_id,
            kind,
            owner_id: Some(owner_player_id),
            location: Some((x, y)),
            traits: vec![
                TraitState::BodyOrientation { quantized_facings: 32 },
                TraitState::Mobile {
                    facing, from_cell: cell, to_cell: cell,
                    center_position: center,
                },
                TraitState::Health { hp },
            ],
            activity: None,
        };
        self.actors.insert(unit_id, actor);
        eprintln!("SPAWN: {} id={} at ({},{}) owner={} speed={} hp={}",
            unit_type, unit_id, x, y, owner_player_id, speed, hp);
    }

    /// Find a spawn location near a production building for the given owner.
    fn find_spawn_location(&self, owner_player_id: u32, unit_type: &str) -> Option<(i32, i32)> {
        let is_infantry = matches!(unit_type, "e1" | "e2" | "e3" | "e4" | "e6" | "e7" | "shok"
            | "medi" | "mech" | "dog" | "spy" | "thf" | "hijacker");

        // Find production building for this unit type
        for actor in self.actors.values() {
            if actor.owner_id != Some(owner_player_id) { continue; }
            if actor.kind != ActorKind::Building { continue; }

            if let Some((bx, by)) = actor.location {
                // Check if this is the right production building
                let is_right_building = if is_infantry {
                    // TENT (allies) or BARR (soviet) produce infantry
                    true // Simplified: any building can produce for now
                } else {
                    true // WEAP produces vehicles
                };

                if is_right_building {
                    // Find an empty adjacent cell for spawning
                    let (fw, fh) = (3, 2); // Approximate footprint
                    for dy in -1..=fh {
                        for dx in -1..=fw {
                            let sx = bx + dx;
                            let sy = by + dy;
                            if self.terrain.is_passable(sx, sy) {
                                return Some((sx, sy));
                            }
                        }
                    }
                }
            }
        }
        None
    }

    /// Deploy an MCV: remove it and create a Construction Yard.
    fn deploy_transform(&mut self, mcv_actor_id: u32, mcv_location: (i32, i32), owner_player_id: u32) {
        let fact_location = (mcv_location.0 - 1, mcv_location.1 - 1);
        eprintln!("DEPLOY: removing MCV actor {} at {:?}, creating FACT at {:?}",
            mcv_actor_id, mcv_location, fact_location);

        // Remove MCV
        self.actors.remove(&mcv_actor_id);

        // Create Construction Yard (FACT) with new actor ID
        let fact_id = self.next_actor_id;
        self.next_actor_id += 1;

        let top_left = CPos::new(fact_location.0, fact_location.1);
        // FACT ISync traits in construction order:
        //   0. BodyOrientation (QuantizedFacings=1)
        //   1. Building (TopLeft)
        //   2. Health (HP=150000)
        //   3. RevealsShroud (base class fields invisible → 0)
        //   4. RevealsShroud@GAPGEN (same → 0)
        //   5. FrozenUnderFog (VisibilityHash=0, updated below)
        //   6. RepairableBuilding (RepairersHash=0)
        //   7. ConyardChronoReturn (all zero)
        let fact_actor = Actor {
            id: fact_id,
            kind: ActorKind::Building,
            owner_id: Some(owner_player_id),
            location: Some(fact_location),
            traits: vec![
                TraitState::BodyOrientation { quantized_facings: 1 },
                TraitState::Building { top_left },
                TraitState::Health { hp: 150000 },
                TraitState::RevealsShroud,
                TraitState::RevealsShroud, // @GAPGEN
                TraitState::FrozenUnderFog { visibility_hash: 0 },
                TraitState::RepairableBuilding { repairers_hash: 0 },
                TraitState::ConyardChronoReturn,
            ],
            activity: None,
        };
        self.actors.insert(fact_id, fact_actor);

        // Occupy terrain (FACT is 3x2)
        self.terrain.occupy_footprint(fact_location.0, fact_location.1, 3, 2, fact_id);

        eprintln!("DEPLOY: created FACT actor {} at {:?} TopLeft.bits={}",
            fact_id, fact_location, top_left.bits);

        // === Side effects from subsequent ticks within the same NetFrameInterval ===

        // 1. Re-enable PQ@Building and PQ@Defense for owning player.
        if let Some(owner) = self.actors.get_mut(&owner_player_id) {
            for t in &mut owner.traits {
                if let TraitState::ClassicProductionQueue { pq_type, enabled, .. } = t {
                    if *pq_type == PqType::Building || *pq_type == PqType::Defense {
                        *enabled = true;
                    }
                }
            }
        }

        // 2. Update FrozenActorLayer for ALL players.
        let everyone_id = self.everyone_player_id;
        let player_ids = self.player_actor_ids.clone();
        for &pid in &player_ids {
            if let Some(player) = self.actors.get_mut(&pid) {
                for t in &mut player.traits {
                    if let TraitState::FrozenActorLayer { frozen_hash, visibility_hash } = t {
                        *frozen_hash = frozen_hash.wrapping_add(fact_id as i32);
                        let can_see = pid == owner_player_id || pid == everyone_id;
                        if !can_see {
                            *visibility_hash = visibility_hash.wrapping_add(fact_id as i32);
                        }
                        break;
                    }
                }
            }
        }

        // 3. Update FrozenUnderFog VisibilityHash on FACT.
        let visibility_hash = self.compute_frozen_visibility_hash(owner_player_id);
        if let Some(fact) = self.actors.get_mut(&fact_id) {
            for t in &mut fact.traits {
                if let TraitState::FrozenUnderFog { visibility_hash: vh } = t {
                    *vh = visibility_hash;
                    break;
                }
            }
        }
    }

    /// Compute FrozenUnderFog VisibilityHash for an actor visible to owner and Everyone.
    fn compute_frozen_visibility_hash(&self, owner_player_id: u32) -> i32 {
        let mut hash = 0i32;
        for &pid in self.player_actor_ids.iter().rev() {
            let visible = pid == owner_player_id || pid == self.everyone_player_id;
            hash = hash * 2 + if visible { 1 } else { 0 };
        }
        hash
    }
}

/// Tick facing toward target by step, matching C#'s Util.TickFacing(WAngle).
fn tick_facing(facing: i32, desired: i32, step: i32) -> i32 {
    let left_turn = ((facing - desired) % 1024 + 1024) % 1024;
    if left_turn < step {
        return desired;
    }
    let right_turn = ((desired - facing) % 1024 + 1024) % 1024;
    if right_turn < step {
        return desired;
    }
    if right_turn < left_turn {
        ((facing + step) % 1024 + 1024) % 1024
    } else {
        ((facing - step) % 1024 + 1024) % 1024
    }
}

/// Convert a cell position to world position (rectangular grid).
pub fn center_of_cell(x: i32, y: i32) -> WPos {
    WPos::new(1024 * x + 512, 1024 * y + 512, 0)
}

/// Parse "X,Y" cell target from order target_string.
fn parse_cell_target(s: &str) -> Option<(i32, i32)> {
    let mut parts = s.split(',');
    let x = parts.next()?.trim().parse::<i32>().ok()?;
    let y = parts.next()?.trim().parse::<i32>().ok()?;
    Some((x, y))
}

/// Get building footprint dimensions and HP for known building types.
/// Returns (width, height, hp).
fn building_footprint(building_type: &str) -> (i32, i32, i32) {
    match building_type {
        "powr" => (2, 2, 40000),
        "apwr" => (2, 2, 70000),
        "tent" | "barr" => (2, 2, 50000),
        "weap" | "weap.ukraine" => (3, 2, 100000),
        "proc" => (3, 2, 90000),
        "fact" => (3, 2, 150000),
        "fix" => (3, 2, 80000),
        "dome" => (2, 2, 60000),
        "hpad" | "afld" => (2, 2, 80000),
        "spen" | "syrd" => (3, 3, 120000),
        "atek" | "stek" => (2, 2, 60000),
        "pbox" | "hbox" | "gun" | "ftur" | "tsla" | "agun" | "sam" | "gap" => (1, 1, 40000),
        _ => (2, 2, 50000), // Default
    }
}

/// Get production cost for any buildable item.
fn production_cost(name: &str) -> i32 {
    match name {
        // Buildings
        "powr" => 300, "apwr" => 500,
        "tent" | "barr" => 400,
        "weap" | "weap.ukraine" => 2000,
        "proc" => 1400, "fix" => 1200,
        "dome" | "atek" | "stek" => 2800,
        "hpad" | "afld" => 500,
        "spen" | "syrd" => 650,
        "pbox" => 400, "hbox" => 600, "gun" => 600,
        "ftur" => 600, "tsla" => 1500, "agun" => 600,
        "sam" => 750, "gap" => 500,
        // Infantry
        "e1" => 100, "e2" => 160, "e3" => 300, "e4" => 200,
        "e6" => 500, "e7" => 600, "shok" => 400,
        "medi" => 600, "mech" => 500,
        "dog" => 200, "spy" => 500, "thf" => 500,
        // Vehicles
        "1tnk" => 700, "2tnk" => 800, "3tnk" => 1500, "4tnk" => 1800,
        "v2rl" => 700, "arty" => 600, "harv" => 1400,
        "mcv" => 2500, "apc" => 800, "jeep" => 600,
        "mnly" => 500, "ttnk" => 1500, "ctnk" => 2000,
        // Aircraft
        "heli" => 1200, "hind" => 1200, "mig" => 2000, "yak" => 800,
        // Naval
        "ss" => 950, "msub" => 1800, "sub" => 950,
        "dd" => 1000, "ca" => 2000, "pt" => 700,
        _ => {
            eprintln!("PRODUCTION: unknown item cost for '{}'", name);
            0
        }
    }
}

/// Check if an item name is a unit (vs building).
fn is_unit_type(name: &str) -> bool {
    matches!(name,
        "e1" | "e2" | "e3" | "e4" | "e6" | "e7" | "shok" | "medi" | "mech" | "dog" |
        "spy" | "thf" | "hijacker" |
        "1tnk" | "2tnk" | "3tnk" | "4tnk" | "v2rl" | "arty" | "harv" | "mcv" | "mnly" |
        "apc" | "truk" | "jeep" | "ttnk" | "ctnk" | "qtnk" | "stnk" | "dtrk" |
        "heli" | "hind" | "mig" | "yak" | "tran" |
        "ss" | "msub" | "sub" | "dd" | "ca" | "pt" | "lst"
    )
}

/// Get unit stats: (kind, speed, hp).
fn unit_stats(unit_type: &str) -> (ActorKind, i32, i32) {
    match unit_type {
        // Infantry
        "e1" => (ActorKind::Infantry, 43, 50000),
        "e2" => (ActorKind::Infantry, 43, 50000),
        "e3" => (ActorKind::Infantry, 43, 45000),
        "e4" => (ActorKind::Infantry, 43, 60000),
        "e6" => (ActorKind::Infantry, 43, 25000),
        "e7" => (ActorKind::Infantry, 43, 100000),
        "shok" => (ActorKind::Infantry, 43, 80000),
        "medi" => (ActorKind::Infantry, 43, 80000),
        "mech" => (ActorKind::Infantry, 43, 70000),
        "dog" => (ActorKind::Infantry, 85, 20000),
        "spy" => (ActorKind::Infantry, 56, 25000),
        "thf" => (ActorKind::Infantry, 56, 50000),
        // Vehicles
        "1tnk" => (ActorKind::Vehicle, 113, 160000),
        "2tnk" => (ActorKind::Vehicle, 85, 260000),
        "3tnk" => (ActorKind::Vehicle, 71, 400000),
        "4tnk" => (ActorKind::Vehicle, 56, 500000),
        "v2rl" => (ActorKind::Vehicle, 71, 150000),
        "arty" => (ActorKind::Vehicle, 85, 75000),
        "harv" => (ActorKind::Vehicle, 56, 60000),
        "mcv" => (ActorKind::Vehicle, 56, 60000),
        "apc" => (ActorKind::Vehicle, 113, 200000),
        "jeep" => (ActorKind::Vehicle, 113, 150000),
        "mnly" => (ActorKind::Vehicle, 85, 55000),
        _ => (ActorKind::Vehicle, 71, 100000), // Default
    }
}

/// Assign spawn points to playable players using the playerRandom sequence.
fn assign_spawn_points(
    spawn_locations: &[(i32, i32)],
    num_playable: usize,
    seed: i32,
    map_players: &[openra_data::oramap::PlayerDef],
) -> Vec<(i32, i32)> {
    let mut player_rng = MersenneTwister::new(seed);

    // Non-playable players: ResolveFaction for each.
    // "allies" faction has no RandomFactionMembers → no RNG consumption.
    for p in map_players {
        if !p.playable {
            // nothing to do
        }
    }

    let mut available_spawns: Vec<usize> = (0..spawn_locations.len()).collect();
    let mut assignments = Vec::new();

    for i in 0..num_playable {
        // ResolveFaction("Random"): 2 playerRandom calls
        let meta_faction = player_rng.next_range(0, 2);
        eprintln!("playerRNG[{}]: meta_faction={} rng.last={} total={}",
            i, meta_faction, player_rng.last, player_rng.total_count);

        if meta_faction == 0 {
            let specific = player_rng.next_range(0, 3);
            eprintln!("playerRNG[{}]: specific_allies={} rng.last={} total={}",
                i, specific, player_rng.last, player_rng.total_count);
        } else {
            let specific = player_rng.next_range(0, 2);
            eprintln!("playerRNG[{}]: specific_soviet={} rng.last={} total={}",
                i, specific, player_rng.last, player_rng.total_count);
        }

        // AssignHomeLocation
        if i == 0 {
            let idx = player_rng.next_range(0, available_spawns.len() as i32) as usize;
            eprintln!("playerRNG[{}]: spawn_idx={} from {} available, rng.last={} total={}",
                i, idx, available_spawns.len(), player_rng.last, player_rng.total_count);
            let spawn_idx = available_spawns.remove(idx);
            assignments.push(spawn_locations[spawn_idx]);
        } else {
            let spawn_idx = available_spawns.remove(0);
            assignments.push(spawn_locations[spawn_idx]);
        }
    }

    assignments
}

/// Build ISync traits for a player actor at tick 0.
fn build_player_traits(starting_cash: i32) -> Vec<TraitState> {
    let mut traits = Vec::new();

    // Construction order (dependency-resolved):
    // Initial batch (no Requires):
    //   0. Shroud
    //   1. PlayerResources
    //   2. MissionObjectives
    //   3. DeveloperMode
    //   4. GpsWatcher
    //   5. PlayerExperience
    // Second batch (Requires met):
    //   6-11. ClassicProductionQueue×6
    //   12.   PowerManager
    //   13.   FrozenActorLayer

    traits.push(TraitState::Shroud { disabled: false });
    traits.push(TraitState::PlayerResources { cash: starting_cash, resources: 0, resource_capacity: 0 });
    traits.push(TraitState::MissionObjectives { objectives_hash: 0 });
    traits.push(TraitState::DeveloperMode { flags: [false; 7] });
    traits.push(TraitState::GpsWatcher {
        explored: false, launched: false, granted_allies: false, granted: false,
    });
    traits.push(TraitState::PlayerExperience { experience: 0 });

    for &pq_type in PqType::ALL {
        traits.push(TraitState::ClassicProductionQueue {
            pq_type,
            enabled: true,
            is_valid_faction: true,
        });
    }

    traits.push(TraitState::PowerManager { power_provided: 0, power_drained: 0 });
    traits.push(TraitState::FrozenActorLayer { frozen_hash: 0, visibility_hash: 0 });

    traits
}

/// Build ISync traits for an MCV actor at tick 0.
fn build_mcv_traits(spawn_x: i32, spawn_y: i32, facing: i32) -> Vec<TraitState> {
    let cell = CPos::new(spawn_x, spawn_y);
    let center = center_of_cell(spawn_x, spawn_y);
    vec![
        TraitState::BodyOrientation { quantized_facings: 32 },
        TraitState::Mobile { facing, from_cell: cell, to_cell: cell, center_position: center },
        TraitState::Chronoshiftable { origin: CPos::new(0, 0), return_ticks: 0 },
        TraitState::Health { hp: 60000 },
        TraitState::RevealsShroud,
    ]
}

/// Build a World from parsed map data, game seed, and lobby info.
pub fn build_world(
    map: &openra_data::oramap::OraMap,
    random_seed: i32,
    lobby: &LobbyInfo,
) -> World {
    let rng = MersenneTwister::new(random_seed);
    let mut actors: BTreeMap<u32, Actor> = BTreeMap::new();
    let mut next_id: u32 = 0;

    // === Actor ID 0: World actor ===
    actors.insert(next_id, Actor {
        id: next_id,
        kind: ActorKind::World,
        owner_id: None,
        location: None,
        traits: vec![TraitState::DebugPauseState { paused: true }],
        activity: None,
    });
    next_id += 1;

    // === Player actors ===
    let mut player_actor_ids: Vec<u32> = Vec::new();

    // Non-playable players (Neutral, Creeps, etc.)
    let non_playable: Vec<_> = map.players.iter().filter(|p| !p.playable).collect();
    for _p in &non_playable {
        let id = next_id;
        actors.insert(id, Actor {
            id,
            kind: ActorKind::Player,
            owner_id: None,
            location: None,
            traits: build_player_traits(lobby.starting_cash),
            activity: None,
        });
        player_actor_ids.push(id);
        next_id += 1;
    }

    // Playable players
    for _slot in &lobby.occupied_slots {
        let id = next_id;
        actors.insert(id, Actor {
            id,
            kind: ActorKind::Player,
            owner_id: None,
            location: None,
            traits: build_player_traits(lobby.starting_cash),
            activity: None,
        });
        player_actor_ids.push(id);
        next_id += 1;
    }

    // "Everyone" spectator player
    let everyone_player_id = next_id;
    actors.insert(next_id, Actor {
        id: next_id,
        kind: ActorKind::Player,
        owner_id: None,
        location: None,
        traits: build_player_traits(lobby.starting_cash),
        activity: None,
    });
    player_actor_ids.push(next_id);
    next_id += 1;

    // === Map actors ===
    let mut spawn_locations: Vec<(i32, i32)> = Vec::new();
    let mut mine_count: usize = 0;

    for map_actor in &map.actors {
        let id = next_id;
        next_id += 1;

        let is_tree = map_actor.actor_type.starts_with('t')
            && (map_actor.actor_type.len() == 3 || map_actor.actor_type.starts_with("tc"));
        let is_mine = map_actor.actor_type == "mine";
        let is_spawn = map_actor.actor_type == "mpspawn";

        let top_left = CPos::new(map_actor.location.0, map_actor.location.1);
        let mut trait_list = Vec::new();
        let kind;

        if is_tree {
            kind = ActorKind::Tree;
            trait_list.push(TraitState::BodyOrientation { quantized_facings: 1 });
            trait_list.push(TraitState::Building { top_left });
            trait_list.push(TraitState::Health { hp: 50000 });
        } else if is_mine {
            mine_count += 1;
            kind = ActorKind::Mine;
            trait_list.push(TraitState::BodyOrientation { quantized_facings: 1 });
            trait_list.push(TraitState::Building { top_left });
        } else if is_spawn {
            spawn_locations.push(map_actor.location);
            kind = ActorKind::Spawn;
            let center = center_of_cell(map_actor.location.0, map_actor.location.1);
            trait_list.push(TraitState::Immobile { top_left, center_position: center });
            trait_list.push(TraitState::BodyOrientation { quantized_facings: 1 });
        } else {
            kind = ActorKind::World; // Unknown actor type
        }

        actors.insert(id, Actor {
            id,
            kind,
            owner_id: None,
            location: Some(map_actor.location),
            traits: trait_list,
            activity: None,
        });
    }

    // === Starting units (MCVs) ===
    let player_spawn_assignments = assign_spawn_points(
        &spawn_locations,
        lobby.occupied_slots.len(),
        random_seed,
        &map.players,
    );

    let facing = 512; // BaseActorFacing default
    let num_non_playable = non_playable.len();
    for (pi, &(spawn_x, spawn_y)) in player_spawn_assignments.iter().enumerate() {
        let owner_pid = player_actor_ids[num_non_playable + pi];
        eprintln!("MCV[{}] spawn=({},{}) facing={} owner={}", pi, spawn_x, spawn_y, facing, owner_pid);
        let id = next_id;
        actors.insert(id, Actor {
            id,
            kind: ActorKind::Mcv,
            owner_id: Some(owner_pid),
            location: Some((spawn_x, spawn_y)),
            traits: build_mcv_traits(spawn_x, spawn_y, facing),
            activity: None,
        });
        next_id += 1;
    }

    // Initialize terrain map and mark existing buildings/trees as occupied.
    let mut terrain = TerrainMap::new(map.map_size.0, map.map_size.1);
    for actor in actors.values() {
        if let Some((x, y)) = actor.location {
            match actor.kind {
                ActorKind::Tree | ActorKind::Mine => {
                    terrain.set_occupant(x, y, actor.id);
                }
                ActorKind::Building => {
                    // Default 2x2 footprint for initial buildings
                    terrain.occupy_footprint(x, y, 2, 2, actor.id);
                }
                _ => {}
            }
        }
    }

    World {
        actors,
        synced_effects: Vec::new(),
        rng,
        paused: true,
        world_tick: 0,
        frame_number: 0,
        order_latency: 15,
        next_actor_id: next_id,
        frame_end_tasks: Vec::new(),
        player_actor_ids,
        everyone_player_id,
        mine_count,
        seeds_resource_ticks: 0,
        production: HashMap::new(),
        terrain,
        map_width: map.map_size.0,
        map_height: map.map_size.1,
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
    fn shroud_false_disabled() {
        let t = TraitState::Shroud { disabled: false };
        assert_eq!(t.sync_hash(), 0);
    }

    #[test]
    fn player_resources_5000_cash() {
        let t = TraitState::PlayerResources { cash: 5000, resources: 0, resource_capacity: 0 };
        assert_eq!(t.sync_hash(), 5000);
    }
}
