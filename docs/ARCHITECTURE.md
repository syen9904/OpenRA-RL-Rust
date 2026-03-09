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

### Sprite 資產

OpenRA repo 的 `mods/ra/bits/` 包含 ~100 個 .shp sprite 檔 + 地形 tileset（.tem/.sno/.des），共 ~2.2 MB。這些是 OpenRA 團隊自製的 GPL 授權資產（不是 EA 原版），可以合法打包進 WASM bundle。完整的原版高解析度圖需要用戶自己安裝，但基本 sprites 足夠渲染 replay。

動畫定義在 `mods/ra/sequences/*.yaml`（哪些 frame 是走路、攻擊、死亡等）。

---

## .orarep 格式

Command-based replay（存 orders，不存狀態）。二進制格式：

```
重複 N 次:
  ├── ClientID     (int32)
  ├── PacketLength (int32)
  └── PacketData
      ├── Frame number (int32)
      └── Orders[]
          ├── OrderType (byte): 0x65=Fields, 0xFF=Handshake, ...
          ├── OrderString (null-terminated or length-prefixed)
          ├── Flags (byte): HasSubject, HasTarget, HasExtraData, ...
          ├── SubjectID (uint32, if HasSubject)
          ├── TargetActorID or TargetPos (if HasTarget)
          └── ExtraData (uint32, if HasExtraData)

檔案尾端:
  MetaStartMarker (固定 byte sequence)
  Metadata YAML (mod, version, map, players, outcome)
  MetaEndMarker

GroupedOrders: 同一個 order 套用到多個 unit，PacketData 裡帶 subject list。
```

關鍵參考：`ReplayConnection.cs`（解析）、`Order.cs`（序列化/反序列化）。

重播 = 讀 orders → 餵給 engine → engine 重跑遊戲邏輯 → 渲染。要求確定性。

---

## 架構

### Crate 結構

```
openra-sim/       核心模擬（零外部依賴）
├── lib.rs        GameSimulation::new(), tick(), apply_order(), snapshot()
├── state.rs      WorldState, Actor, Player
├── rules.rs      RA 單位/武器數值（從 mods/ra/rules/*.yaml 用腳本生成）
├── math.rs       WPos, WAngle, 定點數
├── rng.rs        MersenneTwister (複製 C#)
└── systems/      移動、攻擊、生產、尋路...

openra-data/      檔案解析
├── orarep.rs     .orarep 解析（二進制 order stream + metadata）
├── oramap.rs     .oramap 載入（zip 包：map.yaml + map.bin terrain tiles + actorslist）
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
    units:       [(id, type, owner, pos, hp, facing, activity, anim_frame)]
    buildings:   [(id, type, owner, pos, hp, production_state, size)]
    projectiles: [(type, pos, target_pos, facing, anim_frame)]
    effects:     [(type, pos, frame)]
    players:     [(cash, power_provided, power_drained, kills)]
    terrain:     tile grid (type per cell, 給渲染用)
    shroud
}
```

注意：WorldState 必須包含渲染所需的所有資訊（owner → 玩家顏色、anim_frame → 動畫幀、size → 建築佔格）。如果渲染時發現缺欄位，要回頭在模擬層的 snapshot() 裡補。

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

**Layer 2（openra-sim）**：Golden Snapshot（由 Track B 產出）。Rust 跑同一場 replay 比對：

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
| **Golden dump 做不出來**（Track B blocker） | 已確認 SyncReport 機制可用，主要風險是 headless 模式 + RL replay 的版本相容性。由於改的是 OpenRA-RL fork，版本檢查可以 patch |
| 不知道 replay 觸發了哪些 Order/Activity | 增量式：碰到不認識的 → skip + 警告 → 補實作 |
| 真實 replay 觸發未列出的邏輯（超武、老兵、crates、海空軍） | 先鎖定一場最簡單的 replay（純步兵互打），再擴展 |
| .oramap 格式比預期複雜（terrain types 影響移動速度、bridge 等特殊地形） | 先用最簡單的地圖，忽略特殊地形 |
| rules 數值抄錯 | 用腳本從 YAML 自動生成，不手抄 |
| WorldState 缺渲染所需欄位（做到 Phase 3 才發現） | Phase 2 就寫好完整的 snapshot()，包含 owner/anim_frame/size |

**Scope creep 策略：降級不 crash**。碰到不認識的 Order → skip，不認識的 Activity → unit 變 idle。先讓整場跑完，再補缺失。

---

## 開發計畫（4 Track 並行）

4 條 track 可以同時推進，只有 Track C 的 golden test 驗證依賴 Track B 的輸出：

```
Track A: openra-data          Track B: Golden Dump (C#)
  .orarep / .oramap 解析        改 World.Tick() 加 dump hook
  SHP sprite 解碼               headless 跑 replay 導出 JSON
  palette 載入                   rules 提取腳本 (YAML → Rust)
  (零依賴，馬上開始)                    │
        │                              │
        ▼                              ▼
Track C: openra-sim            Track D: openra-wasm
  math, rng (不依賴 golden)      WebGL sprite renderer
  rules (依賴 B 的腳本)          地圖渲染 + 動畫
  Move, Attack, Pathfinding      播放 UI + camera
  golden test 驗證 (依賴 B)      (用 mock WorldState 開發)
```

### Track A：openra-data（檔案解析）

無外部依賴，馬上能開始。

1. `.orarep` 二進制解析（Order 反序列化含 flags/grouped orders）
2. `.oramap` 載入（zip：map.yaml + map.bin + actorslist）
3. SHP sprite 解碼、Palette 載入
4. 單元測試：解析已有的 replay 檔，驗證 order 數量、metadata 正確

測試用 replay：`replays/ra-2026-02-20T001259Z.orarep`（13 KB，短局）。

### Track B：Golden Dump（C# 側修改）

產出 golden snapshots，供 Track C 的 golden test 使用。

**核心發現：OpenRA 已有 `SyncReport` 系統**（`Network/SyncReport.cs`），每 tick 可以遍歷所有帶 `[Sync]` 屬性的 actor trait，抓取位置、HP、facing 等欄位值 + `SharedRandom` 的 last/count。不需要從零寫序列化。

**步驟：**

1. **在 `World.Tick()` 加 dump hook**（`World.cs:462` 附近）：
   ```csharp
   WorldTick++;
   // === Golden Dump Hook ===
   if (Environment.GetEnvironmentVariable("GOLDEN_DUMP") != null
       && ShouldDump(WorldTick))
       DumpWorldState(WorldTick);
   ```

2. **`DumpWorldState()` 利用現有 SyncReport 機制**，輸出 JSON：
   ```json
   {
     "tick": 142,
     "random": { "last": 12345, "count": 89 },
     "actors": [
       { "id": 5, "type": "e1", "owner": "Player0",
         "pos": [1024, 2048, 0], "hp": 100, "facing": 64 }
     ]
   }
   ```

3. **Headless 播放 replay**：OpenRA 有 `Game.JoinReplay(path)` API + `NullPlatform` headless 模式。用 Docker 裡的 C# engine 跑：
   ```bash
   GOLDEN_DUMP=1 mono OpenRA.exe \
     --headless \
     --replay=replays/ra-2026-02-20T001259Z.orarep \
     > golden/replay-001.jsonl
   ```

4. **Dump 密度**：前 500 ticks 每 10 ticks，500-5000 每 100 ticks，5000+ 每 500 ticks

5. **rules 提取腳本**：從 `mods/ra/rules/*.yaml` 自動生成 Rust 常數（不手抄）

**輸出**：`tests/golden/*.jsonl` — Rust 的 `cargo test` 讀這些檔案比對。

**風險**：OpenRA headless 模式能不能直接播放 RL replay（`Version: {DEV_VERSION}`）。如果不行，需要 patch 版本檢查邏輯。由於我們改的就是 OpenRA-RL 的 fork，這個風險可控。

### Track C：openra-sim（模擬引擎）

math/rng 可以馬上開始，golden test 驗證等 Track B 輸出。

1. **馬上能做**：WPos/WAngle 定點數、MersenneTwister RNG、rules 數值結構
2. **等 Track B 腳本**：填入實際的 RA 單位/武器數值
3. **等 Track B golden dump**：
   - Move + Attack → Replay 1（步兵互打）golden test pass
   - Pathfinding A*
   - Production + Building + Power
   - Harvester、Shroud
   - Replay 2, 3 → golden test pass

### Track D：openra-wasm（渲染層）

只依賴 WorldState 的 shape（struct 定義），不依賴模擬正確性。用 mock data 開發：

```rust
// 假的 WorldState，幾個 unit 在固定位置
let mock_state = WorldState {
    units: vec![
        Actor { id: 1, type_: "e1", pos: WPos(1024, 2048, 0), .. },
        Actor { id: 2, type_: "2tnk", pos: WPos(3072, 1024, 0), .. },
    ],
    ..
};
```

1. SHP/TMP sprite 解碼 + Palette（共用 Track A 的 openra-data）
2. WebGL batched sprite renderer
3. 地圖 + 單位 + 建築 + 動畫 + 特效 + Fog
4. Camera 控制、wasm-pack 編譯、播放 UI（play/pause/speed）
5. 接上真的 openra-sim 輸出
6. 嵌入 openra-rl.dev

---

## Future Work：Training Runtime

v1 不做 training。但架構設計保證未來可加：

```rust
// 同一個 API，不同的 caller
let mut sim = GameSimulation::new(map, rules);

// Replay (v1): orders 從 .orarep 來，雙方 orders 都預錄好
sim.apply_order(replay_order);
sim.tick();
let state = sim.snapshot(); // → WebGL renderer

// Training (future): RL agent + bot AI 各自產生 orders
sim.apply_order(agent_order);  // RL agent 的行動
sim.apply_order(bot_order);    // 對手 AI 的行動
sim.tick();
let state = sim.snapshot(); // → observation serializer → Python
```

### Training 的前置條件

**對手 AI 是 training 的硬性前提** — 沒有對手就無法訓練。Replay 時雙方 orders 都在 .orarep 裡，不需要 AI。Training 時必須有人即時產生對手的 orders。

三個選項：

| 選項 | 工作量 | 效果 |
|------|--------|------|
| **移植 HackyAI** | 大（幾千行 C# 確定性邏輯） | 跟現有 C# training 行為一致，agent 策略可 transfer |
| **簡化 scripted bot** | 中 | 快速可用，但訓練效果跟 C# 版不同 |
| **Self-play** | 低（不需要 scripted AI） | 兩個 RL agent 對打，但需要訓練框架支援 |

### Training 額外需要的完整清單

| 工作 | 為什麼 replay 不需要 |
|------|---------------------|
| **對手 AI**（上述三選一） | Replay 裡雙方 orders 預錄好 |
| **遊戲初始化** — 從地圖設定建立新遊戲（spawn 玩家、放 MCV、初始化資源） | Replay 的初始狀態由 .orarep + .oramap 決定 |
| **勝負判定** — 偵測 game over 條件 | Replay 跑到最後一個 tick 就停 |
| **完整動作空間**（21 種 action type） | Replay 只需處理 replay 裡出現的 orders |
| **觀測序列化** — WorldState → Python dict | Replay 交給 WebGL |
| **Reward 計算** — 8 維 reward vector | Replay 不需要 |
| **PyO3 binding** | Replay 用 WASM |
| **128 instance 並行調度** | Replay 只跑一場 |

### 解決的問題

消除 JIT crash、gRPC 斷線、128 Docker 容器（~44 GB → ~2.5 GB RAM）。

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
