# OpenRA-RL-Rust: Complete Implementation Plan

## Current State

The upstream repo (yxc20089/OpenRA-RL-Rust) has ~15% of the work done:

**Done:**
- Workspace structure: `openra-sim`, `openra-data`, `openra-wasm`, `openra-train`
- `.orarep` replay parser (binary format, orders, SyncHash extraction, metadata)
- `.oramap` parser (ZIP, map.yaml: players, actors, dimensions)
- MersenneTwister RNG (bit-for-bit match with C#)
- Fixed-point math types: WPos, WVec, WAngle, WDist, CPos
- SyncHash computation framework (identity + trait hashes + RNG + effects + players)
- World struct with initial build_world() for tick 0 (trees, mines, spawns)
- Integration test scaffolding (sync_hash_tick1 — currently disabled, hash doesn't match yet)

**Not done (everything else):**
- Game simulation loop (tick, order dispatch, activity system)
- All gameplay systems (movement, combat, production, harvesting, building, shroud)
- Pathfinding
- YAML rules loading (unit stats, weapons, buildings)
- SHP sprite decoding, palette loading
- WASM rendering layer (WebGL/Canvas)
- Training runtime (PyO3, parallel sims)
- Web frontend (HTML/JS shell)

## Architecture Overview

```
openra-data/          File format parsers (zero game logic)
├── orarep.rs         ✅ .orarep replay parser
├── oramap.rs         ✅ .oramap map parser (needs map.bin terrain)
├── miniyaml.rs       → MiniYaml parser for rules/*.yaml
├── shp.rs            → SHP sprite decoder
├── palette.rs        → Palette loader (.pal files)
└── rules.rs          → Parsed game rules (units, weapons, buildings)

openra-sim/           Deterministic game simulation (zero rendering deps)
├── math.rs           ✅ WPos/WVec/WAngle/WDist/CPos
├── rng.rs            ✅ MersenneTwister
├── sync.rs           ✅ SyncHash computation
├── world.rs          ✅ World struct (needs tick loop, order dispatch)
├── actor.rs          → Actor with trait components
├── activity.rs       → Activity stack (state machine)
├── order.rs          → Order dispatch (Move, Attack, Build, etc.)
├── traits/           → Trait implementations
│   ├── mobile.rs     → Movement + facing
│   ├── health.rs     → HP, damage, death
│   ├── armament.rs   → Weapons, firing
│   ├── building.rs   → Building placement, construction
│   ├── production.rs → Production queues
│   ├── harvester.rs  → Resource gathering
│   ├── shroud.rs     → Fog of war
│   └── power.rs      → Power grid
├── activities/       → Activity implementations
│   ├── move_.rs      → Move to cell (uses pathfinder)
│   ├── attack.rs     → Attack activity
│   ├── harvest.rs    → Harvest cycle
│   └── wait.rs       → Idle/wait
├── pathfinder.rs     → A* pathfinding
├── projectile.rs     → Bullet/missile flight + collision
└── terrain.rs        → Terrain cost map from map.bin

openra-wasm/          Browser replay viewer + live game
├── lib.rs            → WASM entry point (wasm-bindgen)
├── renderer.rs       → WebGL2 sprite renderer
├── camera.rs         → Viewport, scroll, zoom
├── sprites.rs        → SHP → texture atlas
├── ui.rs             → Minimal game UI (minimap, sidebar)
└── web/              → HTML/JS/CSS shell
    ├── index.html
    ├── main.js
    └── style.css

openra-train/         RL training runtime (future, phase 4)
├── lib.rs            → Parallel sim manager
├── env.rs            → Gym-like step/reset API
├── observation.rs    → State → observation tensor
└── pyo3.rs           → Python bindings
```

## Implementation Plan — 4 Phases

### Phase 1: Tick-0 SyncHash Match (Foundation)
**Goal:** `cargo test` passes with tick-0 SyncHash matching the replay exactly.

This requires getting every initial actor's `[Sync]` trait fields right — the exact set of traits, their field values, and their iteration order.

#### 1.1 — Copy upstream repo into our workspace
- Initialize git repo
- Copy all existing code from the reference repo
- Verify `cargo test` compiles and existing tests pass

#### 1.2 — MiniYaml parser (`openra-data/src/miniyaml.rs`)
- Parse OpenRA's tab-indented YAML variant
- Support inheritance (`Inherits: ^BaseUnit`), removal (`Trait: ~`), merge
- Reference: `OpenRA.Game/MiniYaml.cs`

#### 1.3 — Rules loader (`openra-data/src/rules.rs`)
- Parse `mods/ra/rules/*.yaml` into structured data
- Actor definitions: traits list, trait parameters
- Weapon definitions: range, damage, projectile type, warheads
- Reference: `OpenRA.Game/GameRules/Ruleset.cs`, `ActorInfo.cs`

#### 1.4 — Complete `build_world()` with all player traits
- Player actors each have ~13 ISync traits (6x ProductionQueue, PlayerExperience, FrozenActorLayer, GpsWatcher, Shroud, PowerManager, MissionObjectives, DeveloperMode)
- Need exact initial hash values for each
- Reference: `OpenRA.Game/Player.cs`, each trait's `[Sync]` fields

#### 1.5 — Actor/trait component system (`openra-sim/src/actor.rs`)
- Actor struct: ID, ActorInfo ref, trait instances, activity stack
- Trait storage: Vec-based (not HashMap — order matters for SyncHash)
- Interface dispatch via enum or trait objects
- Reference: `OpenRA.Game/Actor.cs`

#### 1.6 — Enable and pass `sync_hash_tick1_matches_replay` test

**Estimated effort:** This is the most critical phase. If tick-0 hash matches, the framework is correct.

---

### Phase 2: Replay Simulation (Core Game Logic)
**Goal:** Simulate a full replay tick-by-tick with SyncHash matching at every tick.

#### 2.1 — World tick loop (`world.rs`)
```rust
pub fn tick(&mut self) {
    self.process_orders();      // dispatch queued orders
    self.tick_activities();     // run each actor's activity stack
    self.tick_projectiles();    // projectile flight + collision
    self.tick_production();     // advance production queues
    self.update_shroud();       // fog of war updates
    self.cleanup();             // remove dead actors, expired effects
}
```
- Reference: `OpenRA.Game/World.cs` tick method, `WorldUtils.cs`

#### 2.2 — Order dispatch (`openra-sim/src/order.rs`)
- Parse order string → route to appropriate trait's `ResolveOrder`
- Key orders: `Move`, `Attack`, `AttackMove`, `Stop`, `Guard`, `Harvest`, `PlaceBuilding`, `StartProduction`, `CancelProduction`, `Sell`, `SetRallyPoint`
- Reference: `OpenRA.Game/Network/Order.cs`, each trait's `IResolveOrder`

#### 2.3 — Activity system (`openra-sim/src/activity.rs`)
- Activity stack per actor (push, pop, cancel, child activities)
- Each activity's `Tick()` returns: continue, done, push child
- Critical: transition timing must be exact (off by 1 tick = desync cascade)
- Reference: `OpenRA.Game/Activities/Activity.cs`

#### 2.4 — Movement system (`traits/mobile.rs` + `activities/move_.rs`)
- Mobile trait: speed, facing, locomotor type, terrain costs
- Move activity: pathfind → follow path cell-by-cell → arrive
- Facing/turning: WAngle rotation per tick
- Reference: `OpenRA.Mods.Common/Traits/Mobile.cs`, `Activities/Move.cs`

#### 2.5 — Pathfinding (`pathfinder.rs`)
- A* on cell grid with terrain costs
- Must match C# tie-breaking exactly (same priority queue behavior)
- Hierarchical pathfinding for large maps
- Reference: `OpenRA.Mods.Common/Pathfinder/PathSearch.cs`, `HierarchicalPathFinder.cs`

#### 2.6 — Combat system (`traits/armament.rs`, `traits/health.rs`, `projectile.rs`)
- Armament: weapon selection, range check, fire rate, burst
- Health: HP tracking, damage application, death
- Projectiles: Bullet (instant/hitscan), Missile (guided)
- Warheads: damage spread, versus armor types
- Reference: `Armament.cs`, `Health.cs`, `Bullet.cs`, `Missile.cs`, `HealthDamage.cs`

#### 2.7 — Production system (`traits/production.rs`)
- Production queues (one per category: Infantry, Vehicle, Building, etc.)
- Build time, cost, prerequisites
- Rally points for spawned units
- Reference: `ProductionQueue.cs`, `Production.cs`

#### 2.8 — Building system (`traits/building.rs`)
- Building placement, footprint, construction progress
- Power grid (PowerManager: provided vs drained)
- Sell/repair
- Reference: `Building.cs`, `PowerManager.cs`

#### 2.9 — Harvester system (`traits/harvester.rs`)
- Resource layer on terrain
- Harvest cycle: find ore → move → harvest → return → unload
- Reference: `Harvester.cs`, `ResourceLayer.cs`

#### 2.10 — Shroud/fog of war (`traits/shroud.rs`)
- Per-player visibility grid
- Revealed by unit sight range
- Reference: `Shroud.cs`

#### 2.11 — Terrain system (`terrain.rs`)
- Parse map.bin for terrain tiles
- Terrain type → movement cost lookup
- Reference: `Map.cs`, `CellLayer.cs`, `TileSet.cs`

#### 2.12 — Full replay SyncHash verification
- Run the test replay through the full simulation
- Compare SyncHash at every tick
- Debug mismatches using component-level hash decomposition
- Incremental: first pass with Move+Attack only, then add systems one by one

**Strategy:** Implement in order of what the test replay exercises. Skip unknown orders gracefully (log warning, don't crash). SyncHash tells you immediately when something is wrong.

---

### Phase 3: Browser Rendering (WASM + WebGL)
**Goal:** Play/watch a replay in the browser with actual sprites and animations.

#### 3.1 — SHP sprite decoder (`openra-data/src/shp.rs`)
- Decode SHP TD format (run-length encoded sprites)
- Output: Vec<Frame> where Frame = width × height × palette indices
- Reference: `OpenRA.Mods.Common/SpriteLoaders/ShpTDLoader.cs`

#### 3.2 — Palette loader (`openra-data/src/palette.rs`)
- Load .pal files (256 × RGB)
- Player color remapping (remap range in palette)
- Reference: `OpenRA.Game/Graphics/HardwarePalette.cs`, `Palette.cs`

#### 3.3 — Sequence definitions
- Parse `mods/ra/sequences/*.yaml` for animation frames
- Map: (unit_type, action, facing) → frame indices in SHP
- Reference: `SequenceProvider.cs`, `DefaultSpriteSequence.cs`

#### 3.4 — Texture atlas generation
- Pack all SHP frames into GPU texture atlases
- Generate at build time or WASM init time
- Output: atlas PNG + UV coordinate map

#### 3.5 — WASM entry point (`openra-wasm/src/lib.rs`)
```rust
#[wasm_bindgen]
pub struct ReplayViewer {
    sim: GameSimulation,
    renderer: Renderer,
    replay: Replay,
    current_tick: i32,
}

#[wasm_bindgen]
impl ReplayViewer {
    pub fn new(replay_bytes: &[u8], map_bytes: &[u8]) -> Self { ... }
    pub fn tick(&mut self) { ... }
    pub fn render(&self, canvas_id: &str) { ... }
    pub fn seek(&mut self, tick: i32) { ... }
    pub fn set_speed(&mut self, multiplier: f32) { ... }
}
```

#### 3.6 — WebGL2 renderer (`openra-wasm/src/renderer.rs`)
- Isometric tile rendering (terrain layer)
- Sprite batching (units, buildings, effects)
- Palette shader (index → RGB in fragment shader, like original OpenRA)
- Layer ordering: terrain → shadows → buildings → units → effects → UI

#### 3.7 — Camera/viewport (`openra-wasm/src/camera.rs`)
- Isometric projection matching OpenRA's grid
- Pan (drag/arrow keys), zoom
- Minimap click-to-navigate

#### 3.8 — Web shell (`openra-wasm/web/`)
- HTML page with canvas element
- JS: load WASM, handle file input (.orarep + .oramap)
- Controls: play/pause, speed, seek bar, player perspective toggle
- Could also host on GitHub Pages or similar

#### 3.9 — MVP: Colored rectangles first
- Before sprites are ready, render units as colored rectangles
- Position from WorldState, color from player
- Proves the sim→render pipeline works end to end

---

### Phase 4: Training Runtime + Live Play (Future)
**Goal:** RL agents train against bots; humans can play in the browser.

#### 4.1 — Bot AI
- Port HackyAI from C# (simple scripted bot)
- Or: implement simplified rush/turtle/balanced strategies
- Self-play option (two RL agents, no hand-coded bot needed)

#### 4.2 — Training runtime (`openra-train/`)
- Parallel simulation manager (128+ games in one process)
- Gym-like API: `reset() → obs`, `step(action) → (obs, reward, done)`
- Observation space: map grid features, unit lists, resource counts
- Action space: 21 discrete actions matching OpenRA-RL's existing action set

#### 4.3 — PyO3 bindings
- Python module wrapping the Rust training runtime
- Direct memory sharing (no gRPC, no serialization overhead)
- Compatible with existing TRL GRPOTrainer pipeline

#### 4.4 — Live play in browser
- WebSocket connection to game server
- Player issues orders via mouse/keyboard → Order structs → sim
- Simple matchmaking (create/join game)
- Bot opponent option for single player

#### 4.5 — Game server
- Lightweight Rust server for multiplayer lockstep
- Order relay + validation
- Replay recording

---

## Key Technical Risks & Mitigations

| Risk | Mitigation |
|------|-----------|
| Activity timing off by 1 tick → desync cascade | Line-by-line C# comparison; each activity unit-tested independently |
| HashMap iteration order (Rust random, C# insertion) | Use `IndexMap` or `Vec` everywhere iteration order matters |
| Integer overflow (C# wraps silently, Rust panics) | Use `wrapping_add`/`wrapping_mul` everywhere |
| A* tie-breaking differs | Match C#'s exact priority queue implementation |
| Sort stability (C# Array.Sort is unstable) | Use `sort_unstable` with same tiebreaker |
| Player trait hashes at tick 0 wrong | Dump C# values with debug logging, compare field-by-field |
| WASM bundle too large | Tree-shake aggressively; sprites as separate lazy-loaded assets |

## Determinism Rules (Must Follow)

1. **No `HashMap` in simulation** — use `IndexMap` or `BTreeMap`
2. **No floats in simulation** — all fixed-point (WPos/WAngle/WDist)
3. **All arithmetic uses wrapping ops** where C# would silently overflow
4. **Actor iteration always by ActorID order**
5. **RNG calls must happen in exact same order as C#**
6. **Test against SyncHash at every tick** — the replay is the oracle

## Immediate Next Steps (What to implement first)

1. **Copy upstream code into this repo** and get `cargo test` passing
2. **MiniYaml parser** — needed by everything that reads game data
3. **Complete player trait hashes** — unblock tick-0 SyncHash match
4. **Actor component system** — foundation for all gameplay
5. **World tick loop + Move activity** — first dynamic SyncHash test
6. **WASM colored-rectangles MVP** — prove the rendering pipeline works in parallel with sim work
