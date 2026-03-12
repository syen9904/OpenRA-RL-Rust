//! OpenRA game simulation engine.
//!
//! Deterministic, zero-dependency core that replays OpenRA (Red Alert) games
//! tick-by-tick. Used by both the browser replay viewer (openra-wasm) and
//! the training runtime (openra-train).

pub mod actor;
pub mod ai;
pub mod math;
pub mod pathfinder;
pub mod rng;
pub mod sync;
pub mod terrain;
pub mod traits;
pub mod world;
