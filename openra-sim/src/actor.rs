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
}

/// An activity queued on an actor (simplified C# Activity system).
#[derive(Debug, Clone)]
pub enum Activity {
    /// Turn toward a target facing at the given speed (WAngle units/tick).
    Turn { target: i32, speed: i32 },
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
