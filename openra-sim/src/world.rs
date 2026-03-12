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
use crate::rng::MersenneTwister;
use crate::sync;
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
    total_cost: i32,
    total_time: i32,
    remaining_cost: i32,
    remaining_time: i32,
    started: bool,
}

impl ProductionItem {
    fn new(cost: i32, build_duration_modifier: i32) -> Self {
        let time = (cost as i64 * build_duration_modifier as i64 / 100) as i32;
        ProductionItem {
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
                    let cost = match item_name.as_str() {
                        "powr" => 300,
                        "apwr" => 500,
                        "tent" | "barr" => 400,
                        "weap" | "weap.ukraine" => 2000,
                        "proc" => 1400,
                        "fix" => 1200,
                        "dome" | "atek" | "stek" => 2800,
                        "hpad" | "afld" => 500,
                        "spen" | "syrd" => 650,
                        _ => {
                            eprintln!("ORDER: StartProduction unknown item '{}' subject={}", item_name, subject_id);
                            0
                        }
                    };
                    if cost > 0 {
                        eprintln!("ORDER: StartProduction subject={} item={} cost={}", subject_id, item_name, cost);
                        let item = ProductionItem::new(cost, 60);
                        self.production.entry(subject_id).or_default().push(item);
                    }
                }
            }
            "StartGame" | "Command" => {}
            other => {
                eprintln!("ORDER: unhandled '{}' subject={:?}", other, order.subject_id);
            }
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

        // Tick production queues: consume cash, advance build time.
        let player_ids: Vec<u32> = self.production.keys().copied().collect();
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
                        items.remove(0);
                        eprintln!("PRODUCTION: item complete for player {}", pid);
                    }
                }
            }
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
            }
        }
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
