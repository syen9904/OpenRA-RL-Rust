//! Game trait state — [VerifySync] fields on C# traits.
//!
//! Each variant of `TraitState` corresponds to a C# trait type with
//! [VerifySync]-marked fields that contribute to the SyncHash computation.
//! The `sync_hash()` method on each variant reproduces the IL-generated
//! hash function from C# Sync.cs exactly.

use crate::math::{CPos, WPos};

/// Bool hash matching C# Sync.cs IL generation quirk.
///
/// Due to a Brtrue instruction quirk, the generated code always leaves
/// the raw bool value (0 or 1) on the stack, not 0xaaa/0x555.
pub(crate) fn hash_bool(b: bool) -> i32 {
    if b { 1 } else { 0 }
}

/// Production queue type (named instance on Player actor).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PqType {
    Building,
    Defense,
    Vehicle,
    Infantry,
    Ship,
    Aircraft,
}

impl PqType {
    /// All production queue types in construction order.
    pub const ALL: &[PqType] = &[
        PqType::Building, PqType::Defense, PqType::Vehicle,
        PqType::Infantry, PqType::Ship, PqType::Aircraft,
    ];
}

/// A game trait instance with its synced state.
///
/// Traits are stored on actors in construction order (dependency-resolved),
/// which determines their position in the SyncHash computation.
#[derive(Debug, Clone)]
pub enum TraitState {
    // === World actor ===
    DebugPauseState { paused: bool },

    // === Player actor traits (in construction order) ===
    Shroud { disabled: bool },
    PlayerResources { cash: i32, resources: i32, resource_capacity: i32 },
    MissionObjectives { objectives_hash: i32 },
    DeveloperMode { flags: [bool; 7] },
    GpsWatcher { explored: bool, launched: bool, granted_allies: bool, granted: bool },
    PlayerExperience { experience: i32 },
    ClassicProductionQueue { pq_type: PqType, enabled: bool, is_valid_faction: bool },
    PowerManager { power_provided: i32, power_drained: i32 },
    FrozenActorLayer { frozen_hash: i32, visibility_hash: i32 },

    // === Unit/building traits ===
    BodyOrientation { quantized_facings: i32 },
    Building { top_left: CPos },
    Health { hp: i32 },
    /// RevealsShroud: base class private fields are NOT visible to reflection
    /// on the derived type, so the hash is always 0.
    RevealsShroud,
    FrozenUnderFog { visibility_hash: i32 },
    RepairableBuilding { repairers_hash: i32 },
    ConyardChronoReturn,
    Mobile { facing: i32, from_cell: CPos, to_cell: CPos, center_position: WPos },
    Chronoshiftable { origin: CPos, return_ticks: i32 },
    Immobile { top_left: CPos, center_position: WPos },
}

impl TraitState {
    /// Compute the [VerifySync] hash for this trait.
    ///
    /// Fields are XOR'd together, matching C#'s IL-generated hash functions.
    /// Fields come before properties in the hash order.
    pub fn sync_hash(&self) -> i32 {
        match self {
            Self::DebugPauseState { paused } => hash_bool(*paused),
            Self::Shroud { disabled } => hash_bool(*disabled),
            Self::PlayerResources { cash, resources, resource_capacity } => {
                cash ^ resources ^ resource_capacity
            }
            Self::MissionObjectives { objectives_hash } => *objectives_hash,
            Self::DeveloperMode { flags } => {
                flags.iter().fold(0i32, |h, &f| h ^ hash_bool(f))
            }
            Self::GpsWatcher { explored, launched, granted_allies, granted } => {
                // Field first, then properties
                hash_bool(*explored) ^ hash_bool(*launched)
                    ^ hash_bool(*granted_allies) ^ hash_bool(*granted)
            }
            Self::PlayerExperience { experience } => *experience,
            Self::ClassicProductionQueue { enabled, is_valid_faction, .. } => {
                // Properties only (no fields)
                hash_bool(*enabled) ^ hash_bool(*is_valid_faction)
            }
            Self::PowerManager { power_provided, power_drained } => {
                power_provided ^ power_drained
            }
            Self::FrozenActorLayer { frozen_hash, visibility_hash } => {
                frozen_hash ^ visibility_hash
            }
            Self::BodyOrientation { quantized_facings } => *quantized_facings,
            Self::Building { top_left } => top_left.bits,
            Self::Health { hp } => *hp,
            Self::RevealsShroud => 0,
            Self::FrozenUnderFog { visibility_hash } => *visibility_hash,
            Self::RepairableBuilding { repairers_hash } => *repairers_hash,
            Self::ConyardChronoReturn => 0,
            Self::Mobile { facing, from_cell, to_cell, center_position } => {
                // Properties: Facing, FromCell, ToCell, CenterPosition
                *facing ^ from_cell.bits ^ to_cell.bits ^ center_position.sync_hash()
            }
            Self::Chronoshiftable { origin, return_ticks } => {
                origin.bits ^ return_ticks
            }
            Self::Immobile { top_left, center_position } => {
                top_left.bits ^ center_position.sync_hash()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{CPos, WPos};

    #[test]
    fn bool_hash_values() {
        assert_eq!(hash_bool(true), 1);
        assert_eq!(hash_bool(false), 0);
    }

    #[test]
    fn debug_pause_state() {
        assert_eq!(TraitState::DebugPauseState { paused: true }.sync_hash(), 1);
        assert_eq!(TraitState::DebugPauseState { paused: false }.sync_hash(), 0);
    }

    #[test]
    fn player_resources() {
        let t = TraitState::PlayerResources { cash: 5000, resources: 0, resource_capacity: 0 };
        assert_eq!(t.sync_hash(), 5000);
    }

    #[test]
    fn developer_mode_all_false() {
        let t = TraitState::DeveloperMode { flags: [false; 7] };
        assert_eq!(t.sync_hash(), 0);
    }

    #[test]
    fn gps_watcher_all_false() {
        let t = TraitState::GpsWatcher {
            explored: false, launched: false, granted_allies: false, granted: false,
        };
        assert_eq!(t.sync_hash(), 0);
    }

    #[test]
    fn production_queue_both_true() {
        let t = TraitState::ClassicProductionQueue {
            pq_type: PqType::Building, enabled: true, is_valid_faction: true,
        };
        assert_eq!(t.sync_hash(), 0); // 1 ^ 1 = 0
    }

    #[test]
    fn production_queue_enabled_false() {
        let t = TraitState::ClassicProductionQueue {
            pq_type: PqType::Building, enabled: false, is_valid_faction: true,
        };
        assert_eq!(t.sync_hash(), 1); // 0 ^ 1 = 1
    }

    #[test]
    fn building_hash_is_cpos_bits() {
        let top_left = CPos::new(5, 10);
        let t = TraitState::Building { top_left };
        assert_eq!(t.sync_hash(), top_left.bits);
    }

    #[test]
    fn mobile_hash() {
        let cell = CPos::new(10, 20);
        let center = WPos::new(10 * 1024 + 512, 20 * 1024 + 512, 0);
        let t = TraitState::Mobile {
            facing: 512, from_cell: cell, to_cell: cell, center_position: center,
        };
        // from_cell == to_cell → they cancel in XOR
        assert_eq!(t.sync_hash(), 512 ^ center.sync_hash());
    }

    #[test]
    fn immobile_hash() {
        let top_left = CPos::new(5, 10);
        let center = WPos::new(5 * 1024 + 512, 10 * 1024 + 512, 0);
        let t = TraitState::Immobile { top_left, center_position: center };
        assert_eq!(t.sync_hash(), top_left.bits ^ center.sync_hash());
    }
}
