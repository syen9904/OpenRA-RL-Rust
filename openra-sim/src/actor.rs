//! Actor component — the core entity in the game world.
//!
//! Each actor has an ID, a kind, optional owner and location,
//! a list of traits (in construction order), and an optional activity.

use serde::Serialize;
use crate::traits::TraitState;

/// Actor kind for rendering and classification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum ActorKind {
    World,
    Player,
    Tree,
    Mine,
    Spawn,
    Mcv,
    Building,
    Infantry,
    Vehicle,
    Aircraft,
    Ship,
}

/// An activity queued on an actor (simplified C# Activity system).
#[derive(Debug, Clone)]
pub enum Activity {
    /// Turn toward a target facing at the given speed (WAngle units/tick).
    Turn { target: i32, speed: i32 },
    /// Move along a path of cells at a given speed (world units/tick).
    Move {
        path: Vec<(i32, i32)>,
        path_index: usize,
        speed: i32,
    },
    /// Attack a target actor. Move in range, then fire.
    Attack {
        target_id: u32,
        weapon_range: i32,     // cells
        weapon_damage: i32,    // base damage per shot
        reload_delay: i32,     // ticks between shots
        reload_remaining: i32, // ticks until next shot
        burst: i32,            // shots per reload cycle
        burst_remaining: i32,  // shots left in current burst
    },
    /// Harvest resources: find ore → move → harvest → deliver → repeat.
    Harvest {
        state: HarvestState,
        /// Refinery actor ID to deliver to.
        refinery_id: u32,
        /// Resources carried (ore units).
        carried_ore: i32,
        /// Resources carried (gem units).
        carried_gems: i32,
        /// Capacity (total units).
        capacity: i32,
        /// Movement path (reused for both to-ore and to-refinery).
        path: Vec<(i32, i32)>,
        path_index: usize,
        speed: i32,
        /// Ticks remaining for current harvest action.
        harvest_ticks: i32,
        /// Last harvested cell (for searching nearby).
        last_harvest_cell: Option<(i32, i32)>,
    },
}

/// Sub-state for the Harvest activity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarvestState {
    /// Searching for a resource cell.
    FindingOre,
    /// Moving to a resource cell.
    MovingToOre,
    /// Harvesting at current cell (waiting BaleLoadDelay ticks).
    Harvesting,
    /// Moving to refinery to deliver.
    MovingToRefinery,
    /// Unloading at refinery.
    Unloading,
}

/// A game actor with its traits and current activity.
#[derive(Debug, Clone)]
pub struct Actor {
    pub id: u32,
    pub kind: ActorKind,
    /// Owning player's actor ID (None for World/Player actors).
    pub owner_id: Option<u32>,
    /// Cell location (for positioned actors).
    pub location: Option<(i32, i32)>,
    /// Traits in construction order (determines sync hash order).
    pub traits: Vec<TraitState>,
    /// Current activity (Turn, Move, Attack, etc.).
    pub activity: Option<Activity>,
    /// Actor type name from rules (e.g., "fact", "2tnk", "e1").
    /// Not synced — used for game logic lookups.
    pub actor_type: Option<String>,
    /// If true, this unit was given an AttackMove order and should scan for
    /// enemies while moving (interrupt move to attack, resume when target dies).
    pub attack_move: bool,
    /// Original AttackMove destination — resume movement here after attacking.
    pub attack_move_dest: Option<(i32, i32)>,
}

impl Actor {
    /// Compute sync hashes for all traits.
    pub fn sync_hashes(&self) -> Vec<i32> {
        self.traits.iter().map(|t| t.sync_hash()).collect()
    }

    /// Get player's cash from PlayerResources trait.
    pub fn cash(&self) -> i32 {
        for t in &self.traits {
            if let TraitState::PlayerResources { cash, .. } = t {
                return *cash;
            }
        }
        0
    }

    /// Set player's cash in PlayerResources trait.
    pub fn set_cash(&mut self, new_cash: i32) {
        for t in &mut self.traits {
            if let TraitState::PlayerResources { cash, .. } = t {
                *cash = new_cash;
                return;
            }
        }
    }
}
