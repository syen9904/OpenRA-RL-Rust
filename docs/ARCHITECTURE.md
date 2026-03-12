# OpenRA Rust Engine — 技術方案

## 目標

用 Rust 重寫 OpenRA (Red Alert) 的遊戲模擬引擎，支援三個使用場景：

1. **RL Training Runtime** — 取代 C# engine，128+ 並行模擬，消除 JIT crash / gRPC 斷線
2. **瀏覽器 Replay Viewer** — 純 client-side 播放 `.orarep`，零 server 依賴
3. **真人對打** — WebSocket lockstep 同步，瀏覽器即可遊玩

**設計約束**：模擬和渲染完全解耦。引擎只接收 orders、推進 tick、輸出 state，不知道上層是 WASM renderer、RL agent 還是網路對戰。

---

## 為什麼用 Rust

現有的 training pipeline：Python → gRPC → C# OpenRA headless process

| 問題 | C# 現狀 | Rust 方案 |
|------|---------|-----------|
| 128 並發 | 128 個獨立 .NET process | 128 個 `World` struct，同一 process |
| 記憶體 | 每 process ~100MB+（含 .NET runtime） | 每 World ~5-10MB（無 GC、無 runtime） |
| 穩定性 | GC pause、JIT crash | 無 GC，編譯期記憶體安全 |
| 通訊開銷 | gRPC 序列化 + 網路 | PyO3 直接函數呼叫，零 serialization |
| 部署 | 需要 .NET SDK + 完整 OpenRA | 單一 binary 或 Python wheel |
| 瀏覽器 | 無法（需 Docker + VNC） | 編譯成 WASM 直接跑 |

預期收益：128 Docker 容器（~44 GB）→ 單 process（~2.5 GB）。

---

## 開發時間線

### Phase 0：地基建設（syen9904, 2026-03-09 ~ 03-11）

從零建立 workspace，實作所有解析器和驗證框架：

- `.orarep` replay 二進制解析（orders + SyncHash 提取）
- `.oramap` 地圖解析（players, actors, dimensions）
- MiniYAML 解析器 + Rules loader（trait 繼承、模板合併）
- MersenneTwister RNG（bit-exact 匹配 C#）
- 定點數座標系統（WPos, WVec, WAngle, WDist, CPos）
- SyncHash 兩迴圈算法（identity + trait hashes + RNG + effects + players）
- Actor/Trait 結構、World 初始化
- **驗證結果**：Frames 1-16 SyncHash 完全匹配 C# replay

### Frame 17 瓶頸與方法論教訓（2026-03-10 ~ 03-11）

Frame 17 是遊戲取消暫停後第一個 ITick 執行的 frame。

**我們的做法**：把 SyncHash 當黑盒數字追（「差 480，是哪個 trait 少算了？」）。
逐行讀 C# 追蹤 PQ 邏輯、RNG sweep 測試、嘗試 Docker debug build — 定位了 PQ delta 和 RNG 調用次數，但無法找到剩餘的 480 gap。

**yxc20089 的做法**：拆解 hash 為三個分量分別排查：

```rust
SyncHashDebug {
    identity: i32,  // actor ID 貢獻 → actor 數量對不對
    traits: i32,    // trait hash 貢獻 → 哪個 trait 算錯
    rng_last: i32,  // RNG 狀態 → 消耗次數對不對
}
```

**根本原因：三個問題同時爆發：**

1. **FrozenActorLayer hash 未更新** — 新 actor 創建時每個玩家的 frozen_hash / visibility_hash 都要更新
2. **同一 frame 內的連鎖效應** — 基地車展開 → 生產佇列重新啟用 → FrozenUnderFog 設定
3. **SeedsResource RNG 消耗** — 7 個礦場 × 每個 2 次 = 14 次 RNG

**教訓**：遇到 desync 不要猜，先分層定位問題在 identity/traits/rng 哪一層。

### Phase 1-2：突破 + 完整模擬（yxc20089, 2026-03-11 ~ 03-12）

在 Phase 0 的基礎上，14 commits 內完成：

- 解決 frame 17 desync
- **全部 60 frames（16-75）SyncHash 驗證通過**
- 完整遊戲邏輯：移動、轉向、攻擊、建造、生產、採礦、電力、科技樹
- 戰爭迷霧（per-player shroud grid）
- A* 尋路（8 方向，terrain cost）
- Bot AI 框架（3 狀態 FSM：BuildUp → Producing → Attacking）
- WASM 瀏覽器 viewer（Canvas2D 彩色方塊 MVP）
- 資料驅動遊戲規則（GameRules: ActorStats + WeaponStats，~50 種 RA 單位）
- SHP sprite 解碼 + 調色盤載入

---

## 架構

### Crate 結構

```
openra-data/          檔案格式解析（零遊戲邏輯）
├── orarep.rs         .orarep replay 解析
├── oramap.rs         .oramap 地圖解析
├── miniyaml.rs       MiniYAML 解析器（tab-indented OpenRA 設定格式）
├── rules.rs          Rules loader（trait 繼承、模板合併）
├── shp.rs            SHP sprite 解碼
└── palette.rs        調色盤載入

openra-sim/           確定性遊戲模擬（核心，零外部依賴）
├── world.rs          World state + tick loop + order dispatch + 所有遊戲邏輯
├── actor.rs          Actor struct（id, kind, owner, location, traits, activity）
├── traits.rs         TraitState enum（20 種 [VerifySync] trait）
├── sync.rs           SyncHash 計算（匹配 C# World.SyncHash()）
├── math.rs           定點數座標（WPos, WVec, WAngle, WDist, CPos）
├── rng.rs            MersenneTwister（bit-exact 匹配 C#）
├── terrain.rs        地形格子、移動成本、佔據狀態、資源層
├── pathfinder.rs     A* 尋路（8 方向、deterministic tie-breaking）
├── gamerules.rs      編譯後的遊戲規則（ActorStats, WeaponStats）
└── ai.rs             Bot AI（3 狀態 FSM）

openra-wasm/          瀏覽器前端（WASM + Canvas2D/WebGL）

openra-train/         RL training runtime（見「Training Runtime 設計」章節）
├── action.rs         動作反序列化（Python dict → GameOrder）
├── obs.rs            觀測序列化（WorldSnapshot → Python dict）
├── env.rs            單局 Gym API（reset / step / fast_advance）
├── pool.rs           並行遊戲管理器（128+ 場同時跑）
└── pyo3.rs           Python 綁定（PyO3）
```

### 核心解耦：模擬 ↔ 消費者

```
openra-sim                              openra-wasm / openra-train
┌──────────────────────┐                ┌──────────────────────────┐
│  World               │                │  Renderer / RL Env       │
│  ├─ process_frame()  │  WorldSnapshot │  ├─ 讀 type → 查 sprite  │
│  ├─ tick_actors()    │ ─────────────→ │  ├─ 讀 pos → 定位        │
│  └─ snapshot()       │   (唯一出口)    │  └─ 讀 hp → observation  │
│                      │                │                          │
│  不知道 sprite       │                │  不知道 pathfinding       │
└──────────────────────┘                └──────────────────────────┘
```

### 每 frame 執行順序

```
process_frame(orders):
  1. frame_number += 1
  2. auto-unpause check (frame > orderLatency)
  3. process_orders()        分派 order 到 actor
  4. sync_hash()             計算 SyncHash（驗證用）
  5. if !paused:
       repeat 3 times (NetFrameInterval=3):
         world_tick += 1
         tick_actors()        移動、攻擊、轉向、生產 tick
         execute_frame_end_tasks()  延遲動作（展開基地、生成單位）
       update_shroud()        更新戰爭迷霧
```

**關鍵**：SyncHash 在 tick_actors() **之前**計算。Frame N 的 hash 反映的是 frame N-1 的 tick 結果。

### 模組對應表（Rust ↔ C#）

```
openra-data:
  orarep.rs       ↔  ReplayConnection.cs + Order.cs + OrderIO.cs
  oramap.rs       ↔  Map.cs
  miniyaml.rs     ↔  MiniYaml.cs
  rules.rs        ↔  Ruleset.cs + ActorInfo.cs
  shp.rs          ↔  ShpTDLoader.cs
  palette.rs      ↔  Palette.cs

openra-sim:
  world.rs        ↔  World.cs
  actor.rs        ↔  Actor.cs
  traits.rs       ↔  各 ISync trait 的 [VerifySync] fields
  sync.rs         ↔  Sync.cs（SyncHash 計算 + IL 生成器）
  math.rs         ↔  WPos.cs / WAngle.cs / CPos.cs
  rng.rs          ↔  MersenneTwister.cs
  terrain.rs      ↔  CellLayer.cs + Map.cs
  pathfinder.rs   ↔  PathSearch.cs + HierarchicalPathFinder.cs
  gamerules.rs    ↔  mods/ra/rules/*.yaml（編譯後形式）
  ai.rs           ↔  HackyAI modules
```

---

## Training Runtime 設計（openra-train）

### 目標架構

```
Python (TRL GRPOTrainer)
  │
  │  from openra_train import PyGamePool
  │  pool = PyGamePool(num_games=128, map_path="...")
  │
  ▼
Layer 5: pyo3.rs        Python ↔ Rust 綁定
  ▼
Layer 4: pool.rs        並行管理 128 場遊戲（rayon）
  ▼
Layer 3: obs.rs         WorldSnapshot → Observation dict
  ▼
Layer 2: action.rs      Action dict → GameOrder
  ▼
Layer 1: env.rs         單局 reset/step wrapper
  ▼
Layer 0: openra-sim     遊戲引擎（已完成 ✅）
```

每層只依賴下面那層，不跨層。每層可獨立測試。

### Layer 1: env.rs — 單局 Gym API

包裝 `World`，提供標準 RL 介面：

```rust
pub struct GameEnv {
    world: World,
    episode_id: String,
}

impl GameEnv {
    pub fn new(map_bytes: &[u8], rules_yaml: &[u8], seed: u32) -> Self;
    pub fn reset(&mut self) -> WorldSnapshot;
    pub fn step(&mut self, orders: &[GameOrder]) -> StepResult;
    pub fn fast_advance(&mut self, ticks: u32) -> StepResult;
}

pub struct StepResult {
    pub snapshot: WorldSnapshot,
    pub reward: f32,
    pub done: bool,
    pub result: GameResult,  // Win / Lose / Draw / Playing
}
```

**測試**：純 Rust unit test — reset 回傳有效 snapshot、step 推進 tick、done 在勝負時觸發。

### Layer 2: action.rs — 動作翻譯

把 training pipeline 的 21 種 action 翻譯成 World 理解的 `GameOrder`：

```rust
pub enum Action {
    Noop,
    Move { actor_id: u32, target_x: i32, target_y: i32 },
    Attack { actor_id: u32, target_id: u32 },
    Build { item_type: String },
    Train { item_type: String },
    Deploy { actor_id: u32 },
    // ... 21 種，對齊 rl_bridge.proto 的 ActionType enum
}

impl Action {
    pub fn to_game_order(&self) -> GameOrder;
}
```

**測試**：純資料轉換 — 每種 action type 轉成正確的 order string 和參數。

### Layer 3: obs.rs — 觀測序列化

把 `WorldSnapshot` 轉成跟 `rl_bridge.proto` 的 `GameObservation` 對齊的結構：

```rust
pub struct Observation {
    pub tick: u32,
    pub economy: Economy,       // cash, ore, power
    pub military: Military,     // kills, losses, army value
    pub units: Vec<UnitInfo>,
    pub buildings: Vec<BuildingInfo>,
    pub visible_enemies: Vec<UnitInfo>,
    pub available_production: Vec<String>,
    pub done: bool,
    pub reward: f32,
    pub result: String,
}

impl Observation {
    pub fn from_step_result(result: &StepResult, player_id: u32) -> Self;
}
```

**測試**：給定 snapshot，驗證 observation 的各欄位正確轉換。

### Layer 4: pool.rs — 並行管理器

同時管理 128 場獨立遊戲，用 rayon 做 CPU 並行：

```rust
pub struct GamePool {
    envs: Vec<GameEnv>,
}

impl GamePool {
    pub fn new(num_games: usize, map_bytes: &[u8], rules_yaml: &[u8]) -> Self;
    pub fn reset_all(&mut self) -> Vec<Observation>;
    pub fn step_all(&mut self, actions: &[Vec<Action>]) -> Vec<StepResult>;
    pub fn reset_one(&mut self, index: usize) -> Observation;
}
```

**測試**：4 場並行獨立性（不同 seed → 不同結果）、step_all 推進所有場次。

### Layer 5: pyo3.rs — Python 綁定

暴露給 Python 的最終 API：

```rust
#[pyclass]
pub struct PyGamePool { pool: GamePool }

#[pymethods]
impl PyGamePool {
    #[new]
    fn new(num_games: usize, map_path: &str) -> PyResult<Self>;
    fn reset_all(&mut self) -> PyResult<Vec<PyObject>>;       // → list[dict]
    fn step_all(&mut self, actions: Vec<PyObject>) -> PyResult<Vec<PyObject>>;
}
```

**測試**：Python pytest — `from openra_train import PyGamePool`，驗證 reset/step 回傳正確格式。

### 實作順序

```
      action.rs ──┐
                  ├──→ env.rs ──→ pool.rs ──→ pyo3.rs
      obs.rs ─────┘
      (可並行)       (等 1+2)    (等 3)      (等 4)
```

Layer 1-2 可同時做（互不依賴）。每完成一層就有可獨立跑的測試。

### 與現有 C# pipeline 的對應

```
現在：  Python → gRPC (網路) → 128 個 C# process (各自 .NET runtime)
目標：  Python → PyO3 (函數呼叫) → 128 個 World struct (同一個 process)
```

Action/Observation 格式對齊 `rl_bridge.proto`，讓 Python 訓練代碼只需改 import，不需改邏輯。

---

## 驗證策略

### 主要手段：SyncHash（replay-as-oracle）

`.orarep` 裡已經存了每 frame 的 SyncHash（OpenRA 自己的 desync 偵測機制）。
不需要修改 C# 原始碼、不需要跑 C# engine，golden data 就在 replay 裡。

```
cargo test
  1. 解析 .orarep → 取出 orders + SyncHash per frame
  2. 用 orders 驅動 Rust 模擬引擎
  3. 每 frame 計算 World.sync_hash()
  4. 比對 Rust hash vs replay hash
  → 全部 match = 模擬正確
  → FAIL at frame 42 = 找差異，修 bug
```

### Debug 方法：分解式排查

| 分量 | 含義 | 不匹配時代表 |
|------|------|------------|
| `identity` | actor ID 貢獻 | actor 數量或 ID 不對 |
| `traits` | trait hash 貢獻 | 某個 trait 的狀態值算錯 |
| `rng_last` | RNG 狀態 | RNG 消耗次數不對 |

### 輔助手段：Scenario test

```rust
#[test] fn mcv_deploy_creates_construction_yard() { ... }
#[test] fn harvester_delivers_resources_to_refinery() { ... }
#[test] fn production_queue_enables_after_building_placed() { ... }
```

---

## 開發流程（必須遵守）

所有引擎邏輯的修改必須遵循以下流程：

```
1. 讀 C# 原始碼 → 理解該功能的確切行為
2. 在 Rust 裡對照實作
3. 跑 SyncHash 驗證（cargo test sync_hash_verify）
4. 通過 → 下一個功能
   不通過 → sync_hash_debug() 分層排查（identity/traits/rng）
```

**絕對不要「猜」遊戲邏輯。** 即使行為看起來很明顯，C# 的實際實作可能有邊界情況或順序依賴。
Phase 0 卡在 frame 17 就是因為沒有嚴格對照 C# 而是靠推測。

**例外：Bot AI（ai.rs）** — Bot 決策不影響 SyncHash，可以自由設計。
但 Bot 調用的引擎 API（movement, combat, production）必須符合 C# 行為。

## 確定性規則

| 規則 | 原因 |
|------|------|
| 不用 HashMap（用 BTreeMap） | Rust HashMap 遍歷順序不確定 |
| 不用浮點數（用定點數 WPos/WAngle/WDist） | 浮點在不同平台有 rounding 差異 |
| 所有算術用 wrapping 操作 | C# 整數溢位靜默 wrap，Rust 會 panic |
| Actor 迭代永遠按 ID 排序 | SyncHash 依賴遍歷順序 |
| RNG 調用順序必須跟 C# 完全一致 | 差一次就 desync |
| 每 frame 都驗 SyncHash | replay 是 oracle |

---

## 已知的 C# 怪行為（踩過的坑）

### Bool hashing（Sync.cs IL 生成器 bug）

C# 的 `[VerifySync]` bool 標記用 IL 生成 hash 函式。
`Ldc_I4 0xaaa` 在 `Brtrue` 之前被 push，導致 branch 永遠被 taken。
結果：`hash(false)=0, hash(true)=1`，0xaaa/0x555 是死碼。

### .NET Reflection 不返回 base class private fields

`GetFields(NonPublic|Instance)` 只返回**當前類別**的 private fields。
`RevealsShroud` 繼承自 `AffectsShroud`，後者的 `[VerifySync]` private fields
在子類上反射不到。結果：**RevealsShroud 的 sync hash 永遠是 0**。

### MersenneTwister.Next(1) 不消耗 RNG

`Next(int high)` 調用 `Next(0, high)`。當 `diff = high - low <= 1` 時，
直接返回 `low` 而不調用底層 RNG。

### FrozenActorLayer 是 per-player 的全局狀態

不是「凍結的 actor」的 trait，而是「玩家能看到的 actor 清單」的 hash。
每當新 actor 被創建，**所有玩家**的 FrozenActorLayer 都要更新。
visibility_hash 只對「看不到該 actor」的玩家累加。

---

## 技術風險

| 風險 | 緩解方式 |
|------|---------|
| Activity timing 差 1 tick → desync | 逐行對照 C#；每個 activity 獨立 unit test |
| A* tie-breaking 不一致 | 匹配 C# 的 priority queue 實作 |
| 128 instances 記憶體超標 | 先 profile 單 instance，再優化 snapshot |
| PyO3 GIL 限制並行 | 用 `Python::allow_threads` 釋放 GIL |

---

## 待完成

| 模組 | 優先順序 | 說明 |
|------|---------|------|
| **Training runtime（openra-train）** | **最高** | 5 層解耦設計，見上方 |
| world.rs 重構 | 中 | 2010 行單檔，training 穩定後再拆 |
| 更多 replay 測試 | 中 | 目前只驗證 1 個 replay |
| WASM viewer 美化 | 低 | 從彩色方塊升級到 SHP sprites |
| 網路對戰 | 低 | WebSocket lockstep 同步 |
| 更多單位類型 | 低 | 海軍、飛機、特殊單位 |
