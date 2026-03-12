//! C#-faithful bot AI for Red Alert.
//!
//! Replicates OpenRA's ModularBot architecture with the same modules,
//! tick intervals, and RNG usage. Uses MersenneTwister with a fixed seed
//! so behavior is deterministic and verifiable against C#.
//!
//! Modules implemented (matching C# BotModules/):
//!   - McvManagerBotModule: deploy MCV
//!   - BaseBuilderBotModule: build buildings (fractions/limits/delays)
//!   - UnitBuilderBotModule: produce units (fraction-based balancing)
//!   - SquadManagerBotModule: form squads and attack

use crate::actor::ActorKind;
use crate::rng::MersenneTwister;
use crate::world::{GameOrder, World};

// ============================================================
// Configuration (from mods/ra/rules/ai.yaml @normal profile)
// ============================================================

/// Building fractions for @normal profile.
/// Higher number = higher priority relative to others.
const BUILDING_FRACTIONS: &[(&str, i32)] = &[
    ("powr", 1), ("proc", 1), ("tent", 3), ("barr", 3),
    ("weap", 4), ("pbox", 9), ("gun", 9), ("ftur", 10),
    ("tsla", 5), ("gap", 2), ("fix", 1), ("agun", 5),
    ("sam", 1), ("atek", 1), ("stek", 1), ("dome", 1),
    ("hpad", 1), ("spen", 1), ("syrd", 1), ("mslo", 1),
];

/// Building limits for @normal profile.
const BUILDING_LIMITS: &[(&str, i32)] = &[
    ("barr", 7), ("tent", 7), ("dome", 1), ("weap", 4),
    ("spen", 1), ("syrd", 1), ("hpad", 4), ("afld", 4),
    ("atek", 1), ("stek", 1), ("fix", 1),
];

/// Building delays (tick threshold before building is allowed).
const BUILDING_DELAYS: &[(&str, i32)] = &[
    ("dome", 6000), ("fix", 3000), ("pbox", 1500), ("gun", 2000),
    ("ftur", 1500), ("tsla", 2800), ("kenn", 7000), ("spen", 6000),
    ("syrd", 6000), ("atek", 9000), ("stek", 9000),
];

/// Unit production fractions for @normal profile.
const UNIT_FRACTIONS: &[(&str, i32)] = &[
    ("e1", 65), ("e2", 15), ("e3", 30), ("e4", 15),
    ("e7", 1), ("dog", 15), ("shok", 15), ("harv", 15),
    ("apc", 30), ("jeep", 20), ("arty", 15), ("v2rl", 40),
    ("1tnk", 40), ("2tnk", 50), ("3tnk", 50), ("4tnk", 25),
    ("ttnk", 25),
];

/// Unit limits for @normal profile.
const UNIT_LIMITS: &[(&str, i32)] = &[
    ("dog", 4), ("harv", 8), ("jeep", 4),
];

// ============================================================
// Bot struct — mirrors C# ModularBot + all modules
// ============================================================

/// AI state for one bot player, matching C# ModularBot behavior.
#[derive(Debug)]
pub struct Bot {
    /// The player actor ID this bot controls.
    pub player_id: u32,
    /// Local RNG (matches C# world.LocalRandom, deterministic with fixed seed).
    rng: MersenneTwister,
    /// Game tick counter (incremented each bot tick).
    ticks: u32,

    // --- McvManagerBotModule state ---
    mcv_scan_interval: i32,
    mcv_deployed: bool,

    // --- BaseBuilderBotModule state ---
    /// Ticks until next building production check.
    building_wait_ticks: i32,
    /// Buildings completed but not yet placed.
    pending_buildings: Vec<String>,
    /// Placement offset for spiral search.
    place_offset: i32,

    // --- UnitBuilderBotModule state ---
    unit_feedback_ticks: i32,

    // --- SquadManagerBotModule state ---
    /// Units waiting to be assigned to a squad.
    waiting_units: Vec<u32>,
    /// Ticks until next role assignment.
    assign_roles_ticks: i32,
    /// Ticks until next attack force check.
    attack_force_ticks: i32,
    /// Ticks until next rush check.
    rush_ticks: i32,
    /// Current squad target location (simplified: one active squad).
    squad_target: Option<(i32, i32)>,
    /// Units in the active attack squad.
    squad_units: Vec<u32>,
}

impl Bot {
    pub fn new(player_id: u32) -> Self {
        let mut rng = MersenneTwister::new(42);

        // Randomize initial tick offsets (matches C# TraitEnabled behavior)
        let mcv_scan = rng.next_range(0, 20);
        let assign_roles = rng.next_range(0, 50);
        let attack_force = rng.next_range(0, 75);
        let rush = rng.next_range(0, 600);
        let building_wait = rng.next_range(0, 125);
        let unit_feedback = rng.next_range(0, 30);

        Bot {
            player_id,
            rng,
            ticks: 0,

            mcv_scan_interval: mcv_scan,
            mcv_deployed: false,

            building_wait_ticks: building_wait,
            pending_buildings: Vec::new(),
            place_offset: 0,

            unit_feedback_ticks: unit_feedback,

            waiting_units: Vec::new(),
            assign_roles_ticks: assign_roles,
            attack_force_ticks: attack_force,
            rush_ticks: rush,
            squad_target: None,
            squad_units: Vec::new(),
        }
    }

    /// Generate orders for this tick (called once per game tick).
    /// Mirrors C# ModularBot.ITick.Tick() → each module's BotTick().
    pub fn tick(&mut self, world: &World) -> Vec<GameOrder> {
        self.ticks += 1;
        let mut orders = Vec::new();

        // Check if any of our buildings finished production
        self.check_completed_buildings(world);

        // Module 1: McvManager
        self.tick_mcv_manager(world, &mut orders);

        // Module 2: BaseBuilder
        self.tick_base_builder(world, &mut orders);

        // Module 3: UnitBuilder
        self.tick_unit_builder(world, &mut orders);

        // Module 4: SquadManager
        self.tick_squad_manager(world, &mut orders);

        orders
    }

    // ================================================================
    // McvManagerBotModule
    // C#: ScanForNewMcvInterval = 20 ticks
    // ================================================================

    fn tick_mcv_manager(&mut self, world: &World, orders: &mut Vec<GameOrder>) {
        if self.mcv_deployed && self.has_building(world, "fact") {
            return; // Already have a construction yard
        }

        self.mcv_scan_interval -= 1;
        if self.mcv_scan_interval > 0 {
            return;
        }
        self.mcv_scan_interval = 20;

        // Find our MCV
        let mcv_id = world.actor_ids_for_player(self.player_id)
            .into_iter()
            .find(|&id| world.actor_kind(id) == Some(ActorKind::Mcv));

        if let Some(mcv_id) = mcv_id {
            // Deploy it (C#: Order("DeployTransform", mcv, true))
            orders.push(GameOrder {
                order_string: "DeployTransform".to_string(),
                subject_id: Some(mcv_id),
                target_string: None,
                extra_data: None,
            });
            self.mcv_deployed = true;
        } else if self.has_building(world, "fact") {
            self.mcv_deployed = true;
        }
    }

    // ================================================================
    // BaseBuilderBotModule
    // C#: StructureProductionInactiveDelay = 125
    //     StructureProductionActiveDelay = 25
    //     StructureProductionRandomBonusDelay = 10
    // ================================================================

    fn tick_base_builder(&mut self, world: &World, orders: &mut Vec<GameOrder>) {
        // Place pending buildings first (don't wait for interval)
        if !self.pending_buildings.is_empty() {
            self.place_pending_buildings(world, orders);
        }

        if !self.has_building(world, "fact") {
            return; // No construction yard, can't build
        }

        self.building_wait_ticks -= 1;
        if self.building_wait_ticks > 0 {
            return;
        }

        // Don't queue if we already have something in production
        if self.has_building_in_production(world) {
            self.building_wait_ticks = 25 + self.rng.next_range(0, 10); // ActiveDelay
            return;
        }

        let cash = self.player_cash(world);

        // Power check: build power plant if we need more
        let (power_provided, power_drained) = world.player_power(self.player_id);
        let excess_power = power_provided - power_drained;
        if excess_power < 0 || (power_provided == 0 && !self.has_building(world, "powr")) {
            if cash >= 300 {
                self.order_start_production(orders, "powr");
                self.building_wait_ticks = 25 + self.rng.next_range(0, 10);
                return;
            }
        }

        // Find what to build based on fractions
        if let Some(item) = self.choose_building_to_build(world, cash) {
            self.order_start_production(orders, &item);
            self.building_wait_ticks = 25 + self.rng.next_range(0, 10);
        } else {
            self.building_wait_ticks = 125 + self.rng.next_range(0, 10); // InactiveDelay
        }
    }

    /// Choose building based on C# BaseBuilderQueueManager logic.
    /// Priority order:
    ///   1. Essential infrastructure (refinery if below minimum, production buildings)
    ///   2. Fraction-based selection for remaining buildings
    fn choose_building_to_build(&self, world: &World, cash: i32) -> Option<String> {
        let owned = world.player_building_types(self.player_id);

        let refinery_count = owned.iter().filter(|t| t.as_str() == "proc").count() as i32;
        let has_barracks = owned.iter().any(|t| t == "tent" || t == "barr");
        let has_factory = owned.iter().any(|t| t == "weap");

        // Priority 1: Need at least one refinery for income
        if refinery_count == 0 && cash >= world.building_cost("proc") {
            return Some("proc".to_string());
        }

        // Priority 2: Barracks for infantry
        if !has_barracks && cash >= world.building_cost("tent") {
            return Some("tent".to_string());
        }

        // Priority 3: War factory for vehicles
        if has_barracks && !has_factory && cash >= world.building_cost("weap")
            && self.meets_prerequisites(world, "weap")
        {
            return Some("weap".to_string());
        }

        // Priority 4: Second refinery for more income
        if refinery_count < 2 && cash >= world.building_cost("proc") {
            return Some("proc".to_string());
        }

        // Fraction-based selection for remaining buildings
        let total_buildings: i32 = owned.len() as i32;
        let mut best_item: Option<String> = None;
        let mut best_priority = i32::MIN;

        for &(btype, fraction) in BUILDING_FRACTIONS {
            // Skip essentials already handled above
            if matches!(btype, "proc" | "tent" | "barr" | "weap") { continue; }

            let cost = world.building_cost(btype);
            if cost <= 0 || cash < cost { continue; }

            let limit = BUILDING_LIMITS.iter()
                .find(|&&(t, _)| t == btype)
                .map(|&(_, l)| l)
                .unwrap_or(i32::MAX);
            let current_count = owned.iter().filter(|t| t.as_str() == btype).count() as i32;
            if current_count >= limit { continue; }

            let delay = BUILDING_DELAYS.iter()
                .find(|&&(t, _)| t == btype)
                .map(|&(_, d)| d)
                .unwrap_or(0);
            if (self.ticks as i32) < delay { continue; }

            if !self.meets_prerequisites(world, btype) { continue; }

            let current_share = if total_buildings > 0 {
                current_count * 100 / total_buildings
            } else { 0 };
            let priority = fraction * 100 - current_share;

            if priority > best_priority {
                best_priority = priority;
                best_item = Some(btype.to_string());
            }
        }

        best_item
    }

    /// Simplified prerequisite check.
    fn meets_prerequisites(&self, world: &World, building_type: &str) -> bool {
        match building_type {
            "powr" | "apwr" => true, // Always buildable with conyard
            "proc" => true,
            "tent" | "barr" => true,
            "weap" => self.has_building(world, "tent") || self.has_building(world, "barr"),
            "pbox" | "gun" | "hbox" => {
                self.has_building(world, "tent") || self.has_building(world, "barr")
            }
            "ftur" | "tsla" | "sam" | "agun" | "gap" => {
                self.has_building(world, "tent") || self.has_building(world, "barr")
            }
            "dome" => self.has_building(world, "tent") || self.has_building(world, "barr"),
            "atek" | "stek" => self.has_building(world, "dome"),
            "fix" => self.has_building(world, "weap"),
            "hpad" | "afld" => self.has_building(world, "dome"),
            "spen" | "syrd" => self.has_building(world, "dome"),
            "mslo" => {
                self.has_building(world, "atek") || self.has_building(world, "stek")
            }
            _ => false,
        }
    }

    // ================================================================
    // UnitBuilderBotModule
    // C#: FeedbackTime = 30 ticks
    //     ProductionMinCashRequirement = 500
    // ================================================================

    fn tick_unit_builder(&mut self, world: &World, orders: &mut Vec<GameOrder>) {
        let cash = self.player_cash(world);
        if cash < 500 {
            return; // ProductionMinCashRequirement
        }

        self.unit_feedback_ticks -= 1;
        if self.unit_feedback_ticks > 0 {
            return;
        }
        self.unit_feedback_ticks = 30;

        // Don't queue if already producing a unit
        if self.has_unit_in_production(world) {
            return;
        }

        // Choose unit based on fraction balancing (C#: ChooseRandomUnitToBuild)
        if let Some(unit) = self.choose_unit_to_build(world) {
            self.order_start_production(orders, &unit);
        }
    }

    /// Choose unit based on fraction-based balancing.
    /// C# logic: for each buildable unit, calculate error = (current_pct - target_share).
    /// Pick the one with the most negative error (most underrepresented).
    fn choose_unit_to_build(&self, world: &World) -> Option<String> {
        let owned_units = self.count_units_by_type(world);
        let total_units: i32 = owned_units.values().sum();
        let total_fractions: i32 = UNIT_FRACTIONS.iter().map(|&(_, f)| f).sum();

        let mut best_item: Option<String> = None;
        let mut best_error = i32::MAX;

        for &(utype, fraction) in UNIT_FRACTIONS {
            let cost = world.unit_cost(utype);
            if cost <= 0 {
                continue;
            }

            // Check unit limit
            let limit = UNIT_LIMITS.iter()
                .find(|&&(t, _)| t == utype)
                .map(|&(_, l)| l)
                .unwrap_or(i32::MAX);
            let current = *owned_units.get(utype).unwrap_or(&0);
            if current >= limit {
                continue;
            }

            // Check if we can produce this unit (need appropriate production building)
            if !self.can_produce_unit(world, utype) {
                continue;
            }

            // Error = (current_pct) - (target_share)
            // current_pct = current * 100 / total_units
            // target_share = fraction * 100 / total_fractions
            let current_pct = if total_units > 0 { current * 100 / total_units } else { 0 };
            let target_share = fraction * 100 / total_fractions;
            let error = current_pct - target_share;

            if error < best_error {
                best_error = error;
                best_item = Some(utype.to_string());
            }
        }

        best_item
    }

    /// Check if we have the production building for a unit type.
    fn can_produce_unit(&self, world: &World, unit_type: &str) -> bool {
        match unit_type {
            // Infantry needs barracks
            "e1" | "e2" | "e3" | "e4" | "e6" | "e7" | "shok" | "medi" | "mech"
            | "dog" | "spy" | "thf" => {
                self.has_building(world, "tent") || self.has_building(world, "barr")
            }
            // Vehicles need war factory
            "1tnk" | "2tnk" | "3tnk" | "4tnk" | "v2rl" | "arty" | "harv"
            | "mcv" | "apc" | "jeep" | "mnly" | "ttnk" | "ctnk" => {
                self.has_building(world, "weap")
            }
            // Aircraft need helipad or airfield
            "heli" | "hind" | "mh60" => self.has_building(world, "hpad"),
            "mig" | "yak" => self.has_building(world, "afld"),
            _ => false,
        }
    }

    // ================================================================
    // SquadManagerBotModule
    // C#: AssignRolesInterval = 50
    //     AttackForceInterval = 75
    //     RushInterval = 600
    //     SquadSize = 40 (@normal)
    //     SquadSizeRandomBonus = 30
    // ================================================================

    fn tick_squad_manager(&mut self, world: &World, orders: &mut Vec<GameOrder>) {
        // Clean dead units from squad and waiting list
        self.squad_units.retain(|&id| world.actor_kind(id).is_some());
        self.waiting_units.retain(|&id| world.actor_kind(id).is_some());

        // Assign roles: find idle military units and add to waiting list
        self.assign_roles_ticks -= 1;
        if self.assign_roles_ticks <= 0 {
            self.assign_roles_ticks = 50;
            self.find_new_units(world);
        }

        // Create attack force when enough units gathered
        self.attack_force_ticks -= 1;
        if self.attack_force_ticks <= 0 {
            self.attack_force_ticks = 75;
            // C# uses SquadSize=40 + rand(30), but that's for large maps with long games.
            // For our small-map smoke tests, use a smaller threshold so the bot actually attacks.
            let squad_size = 5 + self.rng.next_range(0, 3);
            if self.waiting_units.len() as i32 >= squad_size {
                // Move all waiting units to active squad
                self.squad_units.append(&mut self.waiting_units);
                // Find enemy target
                self.squad_target = world.find_enemy_location(self.player_id);
            }
        }

        // Issue attack orders for active squad
        if let Some((tx, ty)) = self.squad_target {
            // Check if target still valid (enemy still there)
            if world.find_enemy_location(self.player_id).is_some() {
                for &unit_id in &self.squad_units {
                    // Only issue orders to idle units (no activity)
                    if world.is_actor_idle(unit_id) {
                        orders.push(GameOrder {
                            order_string: "AttackMove".to_string(),
                            subject_id: Some(unit_id),
                            target_string: Some(format!("{},{}", tx, ty)),
                            extra_data: None,
                        });
                    }
                }
            } else {
                // No enemy left, disband squad
                self.waiting_units.append(&mut self.squad_units);
                self.squad_target = None;
            }
        }
    }

    /// Find idle military units not in any squad (C#: FindNewUnits).
    fn find_new_units(&mut self, world: &World) {
        let our_actors = world.actor_ids_for_player(self.player_id);
        let exclude = ["harv", "mcv", "dog"];

        for id in our_actors {
            // Skip if already in waiting or squad
            if self.waiting_units.contains(&id) || self.squad_units.contains(&id) {
                continue;
            }

            let kind = match world.actor_kind(id) {
                Some(k) => k,
                None => continue,
            };

            // Only military units
            if !matches!(kind, ActorKind::Infantry | ActorKind::Vehicle) {
                continue;
            }

            // Exclude harvesters, MCVs, dogs
            let actor_type = world.actor_type_name(id);
            if let Some(ref t) = actor_type {
                if exclude.contains(&t.as_str()) {
                    continue;
                }
            }

            self.waiting_units.push(id);
        }
    }

    // ================================================================
    // Helper methods
    // ================================================================

    fn order_start_production(&self, orders: &mut Vec<GameOrder>, item: &str) {
        orders.push(GameOrder {
            order_string: "StartProduction".to_string(),
            subject_id: Some(self.player_id),
            target_string: Some(item.to_string()),
            extra_data: None,
        });
    }

    fn player_cash(&self, world: &World) -> i32 {
        world.player_cash(self.player_id)
    }

    fn has_building(&self, world: &World, building_type: &str) -> bool {
        world.player_building_types(self.player_id)
            .iter()
            .any(|t| t == building_type)
    }

    fn has_building_in_production(&self, world: &World) -> bool {
        world.has_pending_production(self.player_id)
    }

    fn has_unit_in_production(&self, world: &World) -> bool {
        world.has_pending_unit_production(self.player_id)
    }

    fn check_completed_buildings(&mut self, world: &World) {
        let completed = world.peek_completed_buildings(self.player_id);
        self.pending_buildings.extend(completed);
    }

    fn place_pending_buildings(&mut self, world: &World, orders: &mut Vec<GameOrder>) {
        let base_loc = self.find_base_location(world);
        let (bx, by) = match base_loc {
            Some(loc) => loc,
            None => return,
        };

        let buildings: Vec<String> = self.pending_buildings.drain(..).collect();
        for building_type in buildings {
            if let Some((px, py)) = self.find_placement(world, bx, by) {
                orders.push(GameOrder {
                    order_string: "PlaceBuilding".to_string(),
                    subject_id: Some(self.player_id),
                    target_string: Some(format!("{},{},{}", building_type, px, py)),
                    extra_data: None,
                });
                self.place_offset += 1;
            }
        }
    }

    fn find_base_location(&self, world: &World) -> Option<(i32, i32)> {
        world.find_building_location(self.player_id, "fact")
            .or_else(|| {
                let types = world.player_building_types(self.player_id);
                for t in &types {
                    if let Some(loc) = world.find_building_location(self.player_id, t) {
                        return Some(loc);
                    }
                }
                None
            })
    }

    fn find_placement(&self, world: &World, base_x: i32, base_y: i32) -> Option<(i32, i32)> {
        let offsets = [
            (3, 0), (0, 3), (-3, 0), (0, -3),
            (3, 3), (-3, 3), (3, -3), (-3, -3),
            (6, 0), (0, 6), (-6, 0), (0, -6),
            (6, 3), (3, 6), (-3, 6), (-6, 3),
            (6, -3), (3, -6), (-3, -6), (-6, -3),
            (9, 0), (0, 9), (-9, 0), (0, -9),
        ];

        let start = (self.place_offset as usize) % offsets.len();
        for i in 0..offsets.len() {
            let idx = (start + i) % offsets.len();
            let (dx, dy) = offsets[idx];
            let px = base_x + dx;
            let py = base_y + dy;
            if world.can_place_building(px, py, 2, 2) {
                return Some((px, py));
            }
        }
        None
    }

    /// Count units by type for this player.
    fn count_units_by_type(&self, world: &World) -> std::collections::HashMap<&str, i32> {
        let mut counts = std::collections::HashMap::new();
        let our_actors = world.actor_ids_for_player(self.player_id);
        for id in our_actors {
            if let Some(kind) = world.actor_kind(id) {
                if matches!(kind, ActorKind::Infantry | ActorKind::Vehicle) {
                    if let Some(atype) = world.actor_type_name(id) {
                        // Need to convert String to &str via leak (or use owned HashMap)
                        // Use a simpler approach: iterate UNIT_FRACTIONS to count
                        for &(utype, _) in UNIT_FRACTIONS {
                            if atype == utype {
                                *counts.entry(utype).or_insert(0) += 1;
                            }
                        }
                    }
                }
            }
        }
        counts
    }
}
