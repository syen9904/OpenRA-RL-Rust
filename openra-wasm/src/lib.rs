//! Browser replay viewer.
//!
//! Compiles to WASM via wasm-pack. Reads .orarep + .oramap files, runs the
//! simulation via openra-sim, and exposes world snapshots as JSON for Canvas2D rendering.

use wasm_bindgen::prelude::*;

use openra_data::{oramap, orarep};
use openra_sim::world::{self, GameOrder, LobbyInfo, SlotInfo};

#[wasm_bindgen(start)]
pub fn init() {
    console_error_panic_hook::set_once();
}

fn lobby_from_replay(replay: &orarep::Replay) -> LobbyInfo {
    let settings = replay.lobby_settings().expect("No lobby settings in replay");
    let occupied_slots = settings
        .occupied_slots
        .iter()
        .map(|(_, player_ref, faction)| SlotInfo {
            player_reference: player_ref.clone(),
            faction: faction.clone(),
        })
        .collect();
    LobbyInfo {
        starting_cash: settings.starting_cash,
        allow_spectators: settings.allow_spectators,
        occupied_slots,
    }
}

/// Orders to skip (lobby/network control, not gameplay).
const SKIP_ORDERS: &[&str] = &[
    "SyncInfo",
    "SyncLobbyClients",
    "SyncLobbySlots",
    "HandshakeResponse",
    "HandshakeRequest",
    "SyncConnectionQuality",
    "FluentMessage",
    "StartGame",
];

#[wasm_bindgen]
pub struct ReplayViewer {
    world: world::World,
    orders: Vec<(i32, orarep::Order)>,
    sync_hashes: Vec<orarep::SyncHashEntry>,
    current_frame: usize,
    max_frame: i32,
}

#[wasm_bindgen]
impl ReplayViewer {
    /// Create a new replay viewer from replay and map file bytes.
    #[wasm_bindgen(constructor)]
    pub fn new(replay_bytes: &[u8], map_bytes: &[u8]) -> Result<ReplayViewer, JsValue> {
        let replay = orarep::parse(replay_bytes)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse replay: {}", e)))?;
        let map = oramap::parse(map_bytes)
            .map_err(|e| JsValue::from_str(&format!("Failed to parse map: {}", e)))?;

        let settings = replay
            .lobby_settings()
            .ok_or_else(|| JsValue::from_str("No lobby settings in replay"))?;
        let lobby = lobby_from_replay(&replay);
        let world = world::build_world(&map, settings.random_seed, &lobby);

        let max_frame = replay
            .sync_hashes
            .last()
            .map(|sh| sh.frame)
            .unwrap_or(0);

        Ok(ReplayViewer {
            world,
            orders: replay.orders,
            sync_hashes: replay.sync_hashes,
            current_frame: 0,
            max_frame,
        })
    }

    /// Advance one network frame. Returns false when the replay is finished.
    pub fn tick(&mut self) -> bool {
        self.current_frame += 1;
        let frame = self.current_frame as i32;

        if frame > self.max_frame {
            return false;
        }

        let orders: Vec<GameOrder> = self
            .orders
            .iter()
            .filter(|(f, o)| {
                *f == frame && !SKIP_ORDERS.contains(&o.order_string.as_str())
            })
            .map(|(_, o)| GameOrder {
                order_string: o.order_string.clone(),
                subject_id: o.subject_id,
                target_string: o.target_string.clone(),
                extra_data: o.extra_data,
            })
            .collect();

        self.world.process_frame(&orders);
        true
    }

    /// Get the current world state as a JSON string.
    pub fn snapshot_json(&self) -> String {
        serde_json::to_string(&self.world.snapshot()).unwrap_or_default()
    }

    /// Get the current network frame number.
    pub fn current_frame(&self) -> i32 {
        self.current_frame as i32
    }

    /// Get the total number of frames in the replay.
    pub fn total_frames(&self) -> i32 {
        self.max_frame
    }
}
