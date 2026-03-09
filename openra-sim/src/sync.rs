//! SyncHash computation — must match World.SyncHash() in World.cs exactly.
//!
//! The hash combines:
//! 1. Actor identity hashes (by ActorID)
//! 2. [Sync]-marked trait field hashes (pos, hp, facing, etc.)
//! 3. Synced effects (projectiles)
//! 4. SharedRandom.Last (RNG state)
//! 5. Player render state
//!
//! Reference: OpenRA.Game/World.cs:502, OpenRA.Game/Sync.cs

use crate::math::{CPos, WAngle, WDist, WPos, WVec};

/// Hash functions matching Sync.cs exactly.
/// These must produce identical results to the C# versions.

/// Sync.HashActor(a) = (int)(a.ActorID << 16)
pub fn hash_actor(actor_id: u32) -> i32 {
    (actor_id << 16) as i32
}

/// Sync.HashPlayer(p) = (int)(p.PlayerActor.ActorID << 16) * 0x567
pub fn hash_player(player_actor_id: u32) -> i32 {
    ((player_actor_id << 16) as i32).wrapping_mul(0x567)
}

/// Sync.HashInt2(i2) = ((i2.X * 5) ^ (i2.Y * 3)) / 4
pub fn hash_int2(x: i32, y: i32) -> i32 {
    ((x * 5) ^ (y * 3)) / 4
}

/// Sync.HashCPos(c) = c.Bits
pub fn hash_cpos(c: CPos) -> i32 {
    c.bits
}

/// WPos.GetHashCode() = X ^ Y ^ Z (used as HashUsingHashCode for WPos)
pub fn hash_wpos(p: WPos) -> i32 {
    p.sync_hash()
}

/// WVec.GetHashCode() = X ^ Y ^ Z
pub fn hash_wvec(v: WVec) -> i32 {
    v.sync_hash()
}

/// WAngle.GetHashCode() = Angle
pub fn hash_wangle(a: WAngle) -> i32 {
    a.sync_hash()
}

/// WDist.GetHashCode() = Length
pub fn hash_wdist(d: WDist) -> i32 {
    d.sync_hash()
}

/// A single trait's sync hash value.
/// Each [Sync]-marked field is XOR'd together.
#[derive(Debug, Clone, Copy)]
pub struct TraitSyncHash {
    pub hash: i32,
}

/// Represents an actor's contribution to the world sync hash.
#[derive(Debug, Clone)]
pub struct ActorSync {
    pub actor_id: u32,
    /// Hashes from all [Sync]-marked traits on this actor
    pub trait_hashes: Vec<i32>,
}

/// Compute World.SyncHash() given the full world state.
///
/// Algorithm from World.cs:502:
/// ```ignore
/// var n = 0;
/// var ret = 0;
/// foreach (var a in Actors)
///     ret += n++ * (int)(1 + a.ActorID) * Sync.HashActor(a);
/// foreach (var actor in ActorsHavingTrait<ISync>())
///     foreach (var syncHash in actor.SyncHashes)
///         ret += n++ * (int)(1 + actor.ActorID) * syncHash.Hash();
/// foreach (var sync in SyncedEffects)
///     ret += n++ * Sync.Hash(sync);
/// ret += SharedRandom.Last;
/// foreach (var p in Players)
///     if (p.UnlockedRenderPlayer)
///         ret += Sync.HashPlayer(p);
/// return ret;
/// ```
pub fn compute_world_sync_hash(
    actor_ids: &[u32],
    actor_syncs: &[ActorSync],
    effect_hashes: &[i32],
    random_last: i32,
    unlocked_render_player_actor_ids: &[u32],
) -> i32 {
    let mut n: i32 = 0;
    let mut ret: i32 = 0;

    // 1. Hash all actors by identity
    for &actor_id in actor_ids {
        ret = ret.wrapping_add(
            n.wrapping_mul((1i32).wrapping_add(actor_id as i32))
                .wrapping_mul(hash_actor(actor_id)),
        );
        n += 1;
    }

    // 2. Hash all [Sync]-marked trait fields
    for actor_sync in actor_syncs {
        for &trait_hash in &actor_sync.trait_hashes {
            ret = ret.wrapping_add(
                n.wrapping_mul((1i32).wrapping_add(actor_sync.actor_id as i32))
                    .wrapping_mul(trait_hash),
            );
            n += 1;
        }
    }

    // 3. Hash synced effects
    for &effect_hash in effect_hashes {
        ret = ret.wrapping_add(n.wrapping_mul(effect_hash));
        n += 1;
    }

    // 4. Hash RNG state
    ret = ret.wrapping_add(random_last);

    // 5. Hash player render state
    for &player_actor_id in unlocked_render_player_actor_ids {
        ret = ret.wrapping_add(hash_player(player_actor_id));
    }

    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hash_actor_values() {
        assert_eq!(hash_actor(0), 0);
        assert_eq!(hash_actor(1), 1 << 16);
        assert_eq!(hash_actor(2), 2 << 16);
    }

    #[test]
    fn hash_player_values() {
        assert_eq!(hash_player(0), 0);
        // (1 << 16) * 0x567 = 65536 * 1383 = 90636288
        assert_eq!(hash_player(1), (1i32 << 16).wrapping_mul(0x567));
    }

    #[test]
    fn hash_int2_values() {
        assert_eq!(hash_int2(0, 0), 0);
        assert_eq!(hash_int2(10, 20), ((10 * 5) ^ (20 * 3)) / 4);
    }

    #[test]
    fn hash_cpos_is_bits() {
        let c = CPos::new(5, 10);
        assert_eq!(hash_cpos(c), c.bits);
    }

    #[test]
    fn empty_world_hash_is_random_last() {
        // No actors, no effects, no players → hash = SharedRandom.Last
        let hash = compute_world_sync_hash(&[], &[], &[], 42, &[]);
        assert_eq!(hash, 42);
    }

    #[test]
    fn single_actor_hash() {
        // One actor with ID=0, no traits
        // n=0: ret += 0 * (1+0) * hash_actor(0) = 0
        // ret += random_last = 100
        let hash = compute_world_sync_hash(&[0], &[], &[], 100, &[]);
        assert_eq!(hash, 100);

        // Actor with ID=1
        // n=0: ret += 0 * (1+1) * hash_actor(1) = 0  (n=0 so always 0)
        // ret += random_last = 100
        let hash = compute_world_sync_hash(&[1], &[], &[], 100, &[]);
        assert_eq!(hash, 100);
    }

    #[test]
    fn two_actors_hash() {
        // Two actors: ID=0, ID=1
        // n=0: ret += 0 * (1+0) * hash_actor(0) = 0
        // n=1: ret += 1 * (1+1) * hash_actor(1) = 1 * 2 * 65536 = 131072
        // ret += random_last = 0
        let hash = compute_world_sync_hash(&[0, 1], &[], &[], 0, &[]);
        assert_eq!(hash, 131072);
    }

    #[test]
    fn actor_with_trait_hash() {
        // Actor ID=0, one trait hash = 999
        // n=0: ret += 0 * (1+0) * hash_actor(0) = 0
        // Then trait hashes, n continues:
        // n=1: ret += 1 * (1+0) * 999 = 999
        // ret += random_last = 0
        let syncs = vec![ActorSync {
            actor_id: 0,
            trait_hashes: vec![999],
        }];
        let hash = compute_world_sync_hash(&[0], &syncs, &[], 0, &[]);
        assert_eq!(hash, 999);
    }

    #[test]
    fn wrapping_arithmetic() {
        // Verify that wrapping_mul/wrapping_add doesn't panic on overflow
        let hash = compute_world_sync_hash(
            &[u32::MAX],
            &[],
            &[],
            i32::MAX,
            &[],
        );
        // Just verify it doesn't panic
        let _ = hash;
    }
}
