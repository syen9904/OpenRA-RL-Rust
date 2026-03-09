# OpenRA Rust Replay Engine — 技術方案

## 目標

在瀏覽器上播放 `.orarep` replay，純 client-side，零 server 依賴。

**為什麼需要**：OpenRA-RL 輸出的 `.orarep` 無法在官方 OpenRA 播放（`Version: {DEV_VERSION}` + `BotType: rl-agent` 不相容），現有 `openra-rl replay` 需要 Docker + VNC。

**設計約束**：模擬和渲染完全解耦，讓同一個模擬核心未來能直接用於 training runtime（取代現有 C# engine 在 GPU cluster 上 128 agent 並行時的 JIT crash、gRPC 斷線、記憶體過高問題）。

---

## 方案：Rust mini replay engine → WASM

只實作 replay 需要的最小遊戲邏輯，不重寫整個 OpenRA。

已排除：C# .NET WASM（bundle ~100-200 MB）、Server-side VNC（需要 server）、State-based replay（缺視覺事件）。

選 Rust 的原因：bundle ~12 MB vs ~100-200 MB、無 runtime 依賴、golden test 可用 `cargo test` 自動化 debug。代價是確定性風險（C# 跟 Rust 的行為必須 bit-for-bit 一致），靠 golden test 解決。

---

## .orarep 格式

Command-based replay（存 orders，不存狀態）：

```
[Frame N] → ClientID + Orders (Move, Attack, Build, ...)
[EOF]     → Metadata YAML (mod, version, map, players, outcome)
```

重播 = 讀 orders → 餵給 engine → engine 重跑遊戲邏輯 → 渲染。要求確定性。

---

## 架構

### Crate 結構

```
openra-sim/       核心模擬（零外部依賴）
├── lib.rs        GameSimulation::new(), tick(), apply_order(), snapshot()
├── state.rs      WorldState, Actor, Player
├── rules.rs      RA 單位/武器數值
├── math.rs       WPos, WAngle, 定點數
├── rng.rs        MersenneTwister (複製 C#)
└── systems/      移動、攻擊、生產、尋路...

openra-data/      檔案解析
├── orarep.rs     .orarep 解析
├── oramap.rs     .oramap 載入
├── shp.rs        SHP sprite 解碼
└── palette.rs    Palette 載入

openra-wasm/      Browser Replay Viewer (v1)
├── lib.rs        WASM bindings
├── renderer.rs   WebGL batched sprite renderer
└── ui.rs         JS interop (play/pause/speed)

openra-train/     Training Runtime (future work, 空 crate)
```

### 核心解耦：模擬 ↔ 渲染

```
openra-sim                              openra-wasm
┌──────────────────────┐                ┌──────────────────────┐
│  GameSimulation      │                │  Renderer            │
│  ├─ apply_order()    │   WorldState   │  ├─ 讀 type → 查 sprite│
│  ├─ tick()           │ ─────────────→ │  ├─ 讀 pos → 定位    │
│  └─ snapshot()       │  (唯一出口)     │  └─ 讀 facing → 選方向│
│                      │                │                      │
│  不知道 sprite       │                │  不知道 pathfinding   │
└──────────────────────┘                └──────────────────────┘
```

**WorldState 是唯一的邊界**：

```
WorldState {
    units:       [(id, type, pos, hp, facing, activity)]
    buildings:   [(id, type, pos, hp, production_state)]
    projectiles: [(type, pos, target_pos)]
    effects:     [(type, pos, frame)]
    players:     [(cash, power, kills)]
    shroud
}
```

為什麼分離：
- golden test 只測模擬，不需要瀏覽器或 WebGL（`cargo test` 能跑）
- 模擬 bug 和渲染 bug 完全隔離
- 未來 training 直接讀 WorldState，不需要渲染層

### 模擬層內部

World 是唯一協調者，各系統是純函數：

```
每 tick:
  1. process_orders()     分派 order 到 unit/building
  2. tick_activities()    每個 unit 跑 activity stack
  3. tick_projectiles()   子彈飛行、碰撞、傷害
  4. tick_production()    生產佇列推進
  5. update_shroud()      更新迷霧
  6. cleanup()            移除死亡 unit、過期 effect
```

### 模組對應表（Rust ↔ C#）

desync 時：看 test fail → 找 Rust 模組 → 找同名 C# 檔 → 逐行對照。

```
openra-data:
  orarep.rs       ↔  ReplayConnection.cs
  order.rs        ↔  Order.cs
  map.rs          ↔  Map.cs
  math.rs         ↔  WPos.cs / WAngle.cs
  rng.rs          ↔  MersenneTwister.cs

openra-sim:
  world.rs        ↔  World.cs
  activity.rs     ↔  Activity.cs
  mobile.rs       ↔  Mobile.cs
  pathfinder.rs   ↔  PathFinder.cs
  armament.rs     ↔  Armament.cs
  projectile.rs   ↔  Bullet.cs / Missile.cs
  health.rs       ↔  Health.cs
  production.rs   ↔  ProductionQueue.cs
  building.rs     ↔  Building.cs
  harvester.rs    ↔  Harvester.cs
  shroud.rs       ↔  Shroud.cs

openra-data (渲染用):
  shp.rs          ↔  ShpTDLoader.cs
  palette.rs      ↔  Palette.cs
```

---

## Activity System — 最難的部分

單位行為是 activity stack（狀態機堆疊），不是簡單的 if-else：

```
Harvester 的生命週期:
FindResources → Move(到礦) → Harvest(20 ticks) → Move(到精煉廠) → Unload → 循環
```

每個 activity 每 tick 可以：繼續、結束（跑下一個）、插入子任務、被外部 cancel。

**為什麼難**：轉換時機差 1 tick = 後面所有行為偏移 = desync 雪崩。

**策略**：先 Move + Attack → golden test pass → 再加 Harvest / Build，增量推進。

---

## 測試策略

**Layer 1（openra-data）**：單元測試。.orarep 解析、座標轉換、RNG 有標準答案。

**Layer 2（openra-sim）**：Golden Snapshot。C# engine 跑 replay dump world state，Rust 跑同一場比對：

```
cargo test
→ FAIL: tick 1523, unit 42 (e1), Y expected 5678 got 5679
→ 讀 mobile.rs，對照 C# Mobile.cs
→ 找到整數除法 rounding 差異 → 修好 → 再跑
```

Snapshot 密度：前 500 ticks 每 10 ticks，500-5000 每 100 ticks，5000+ 每 500 ticks。

**Layer 3（openra-wasm）**：手動看畫面。最後做。

**Debug 原則**：從底往上。Layer 1 錯了 Layer 2 不可能對。Layer 2 對了畫面不對 → 一定是渲染問題。

---

## 潛在問題

### 確定性風險

| 風險 | 怎麼防 |
|------|--------|
| Activity 轉換時機差 1 tick | 逐行對照 C#，每個 activity 單獨 golden test |
| A* tie-breaking | 對照 PathFinder.cs 的 cost 比較邏輯 |
| HashMap 遍歷順序（Rust 隨機，C# 按插入序） | 用 BTreeMap 或 IndexMap |
| 排序穩定性（C# Array.Sort 不穩定） | 用 sort_unstable + 相同 tiebreaker |
| 整數溢位（C# 靜默 wrap，Rust panic） | 用 wrapping_add / wrapping_mul |
| RNG 序列 | 逐行對照 MersenneTwister.cs |

### 工程風險

| 風險 | 怎麼防 |
|------|--------|
| 不知道 replay 觸發了哪些 Order/Activity | 增量式：碰到不認識的 → skip + 警告 → 補實作 |
| 真實 replay 觸發未列出的邏輯（超武、老兵、crates、海空軍） | 先鎖定一場簡單 replay，再擴展 |
| C# golden dump script 的正確性 | dump 後用 C# 自己驗證一次 |

**Scope creep 策略：降級不 crash**。碰到不認識的 Order → skip，不認識的 Activity → unit 變 idle。先讓整場跑完，再補缺失。

---

## 開發計畫

### Phase 1：基礎（1-2 天）

- C# golden dump script + 跑 1 場簡單 replay 產出 snapshots
- openra-data：.orarep 解析、Order 反序列化
- openra-sim：WPos/WAngle 數學、RNG、rules 數值
- 第一批單元測試 pass

### Phase 2：模擬（3-5 天）

- Move + Attack → Replay 1（步兵互打）golden test pass
- Pathfinding A*
- Production + Building + Power
- Harvester
- Shroud
- Replay 2, 3 → golden test pass

### Phase 3：渲染 + 整合（3-5 天）

- SHP/TMP sprite 解碼、Palette
- WebGL batched sprite renderer
- 地圖 + 單位 + 建築 + 動畫 + 特效 + Fog
- Camera 控制、wasm-pack 編譯、播放 UI
- 嵌入 openra-rl.dev

---

## Future Work：Training Runtime

v1 不做 training。但架構設計保證未來可加：

```rust
// 同一個 API，不同的 caller
let mut sim = GameSimulation::new(map, rules);

// Replay (v1): orders 從 .orarep 來
sim.apply_order(replay_order);
sim.tick();
let state = sim.snapshot(); // → WebGL renderer

// Training (future): orders 從 agent API 來
sim.apply_order(agent_order);
sim.tick();
let state = sim.snapshot(); // → observation serializer → Python
```

Training 額外需要：完整動作空間（21 種）、觀測序列化、reward 計算、PyO3 binding、scripted bot AI、128 instance 並行調度。

解決的問題：消除 JIT crash、gRPC 斷線、128 Docker 容器（~44 GB → ~2.5 GB RAM）。

---

## 參考資源

### OpenRA 原始碼（對照用）

```
OpenRA.Game/
  World.cs, Network/ReplayConnection.cs, Network/Order.cs
  Graphics/WorldRenderer.cs, Graphics/SpriteRenderer.cs

OpenRA.Mods.Common/
  Traits/Mobile.cs, Traits/Armament.cs, Traits/Health.cs
  Traits/Building.cs, Traits/Harvester.cs
  Activities/Move.cs, Activities/Attack.cs
  Pathfinder/PathFinder.cs

OpenRA.Mods.Cnc/SpriteLoaders/ShpTDLoader.cs
```
