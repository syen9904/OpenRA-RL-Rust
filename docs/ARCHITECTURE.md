# OpenRA Rust Replay Engine — 技術方案

## 目標

在瀏覽器上播放 `.orarep` replay，純 client-side，零 server 依賴。

**為什麼需要**：OpenRA-RL 輸出的 `.orarep` 無法在官方 OpenRA 播放（`Version: {DEV_VERSION}` + `BotType: rl-agent` 不相容），現有 `openra-rl replay` 需要 Docker + VNC。

**設計約束**：模擬和渲染完全解耦，讓同一個模擬核心未來能直接用於 training runtime（取代現有 C# engine 在 GPU cluster 上 128 agent 並行時的 JIT crash、gRPC 斷線、記憶體過高問題）。

---

## 方案：Rust mini replay engine → WASM

只實作 replay 需要的最小遊戲邏輯，不重寫整個 OpenRA。

選 Rust 的原因：bundle ~12 MB vs C# .NET WASM ~100-200 MB、無 runtime 依賴、golden test 可用 `cargo test` 自動化 debug。代價是確定性風險（C# 跟 Rust 的行為必須 bit-for-bit 一致），靠 SyncHash 驗證。

### Sprite 資產

OpenRA repo 的 `mods/ra/bits/` 包含 ~100 個 .shp sprite 檔 + 地形 tileset（.tem/.sno/.des），共 ~2.2 MB。GPL 授權，可合法打包進 WASM bundle。

動畫定義在 `mods/ra/sequences/*.yaml`（哪些 frame 是走路、攻擊、死亡等）。

---

## .orarep 格式（✅ 已實作）

Command-based replay（存 orders，不存狀態）。二進制格式：

```
重複 N 次:
  ├── ClientID     (int32, -1 = MetaStartMarker → 停止)
  ├── PacketLength (int32)
  └── PacketData
      ├── Frame number (int32)
      └── Orders[]
          ├── OrderType (byte): 0xFF=Fields, 0xFE=Handshake, 0x65=SyncHash, ...
          ├── OrderString (.NET 7-bit length-prefixed UTF-8)
          ├── Flags (int16): Subject|Target|TargetString|Queued|ExtraData|Grouped|...
          └── 條件欄位（根據 flags 讀取）

檔案尾端:
  MetaStartMarker (-1) + Version (1) + YAML metadata + DataLength + MetaEndMarker (-2)

SyncHash packets (OrderType 0x65): 每 tick 一個
  ├── frame     (int32)
  ├── syncHash  (int32)  ← 驗證用的 golden data
  └── defeatState (u64)
```

已驗證：13KB replay 解析出 77 ticks, 166 packets, 22 orders, 75 SyncHash。

---

## 架構

### Crate 結構

```
openra-sim/       核心模擬（零外部依賴）
├── lib.rs        GameSimulation::new(), tick(), apply_order(), snapshot()
├── state.rs      WorldState, Actor, Player
├── rules.rs      RA 單位/武器數值（從 mods/ra/rules/*.yaml 用腳本生成）
├── math.rs       WPos, WAngle, CPos 定點數
├── rng.rs        MersenneTwister (✅ 已實作，8 tests pass)
└── systems/      移動、攻擊、生產、尋路...

openra-data/      檔案解析
├── orarep.rs     .orarep 解析（✅ 已實作，6 tests pass）
├── oramap.rs     .oramap 載入
├── shp.rs        SHP sprite 解碼
└── palette.rs    Palette 載入

openra-wasm/      Browser Replay Viewer (v1)
openra-train/     Training Runtime (future work)
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
    terrain:     tile grid (type per cell)
    shroud
}
```

### 模擬層內部

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
  orarep.rs       ↔  ReplayConnection.cs + Order.cs + OrderIO.cs
  map.rs          ↔  Map.cs
  shp.rs          ↔  ShpTDLoader.cs
  palette.rs      ↔  Palette.cs

openra-sim:
  math.rs         ↔  WPos.cs / WAngle.cs / CPos.cs
  rng.rs          ↔  MersenneTwister.cs
  world.rs        ↔  World.cs
  sync.rs         ↔  Sync.cs（SyncHash 計算）
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

**策略**：先 Move + Attack → SyncHash pass → 再加 Harvest / Build，增量推進。

---

## 驗證策略：SyncHash（不需要改 C# code）

**核心發現：`.orarep` 裡已經存了每 tick 的 SyncHash**。這是 OpenRA 自己的 desync 偵測機制，不需要修改任何 C# 原始碼就能拿到 golden data。

已驗證：13KB 測試 replay 包含 75 個 SyncHash packet（每 tick 一個）。

### SyncHash 計算方式（`World.SyncHash()` in `World.cs:502`）

```csharp
int SyncHash() {
    var n = 0;
    var ret = 0;

    // 1. Hash all actors (by ActorID)
    foreach (var a in Actors)
        ret += n++ * (int)(1 + a.ActorID) * Sync.HashActor(a);

    // 2. Hash all [Sync]-marked trait fields (pos, hp, facing, etc.)
    foreach (var actor in ActorsHavingTrait<ISync>())
        foreach (var syncHash in actor.SyncHashes)
            ret += n++ * (int)(1 + actor.ActorID) * syncHash.Hash();

    // 3. Hash synced effects (projectiles)
    foreach (var sync in SyncedEffects)
        ret += n++ * Sync.Hash(sync);

    // 4. Hash RNG state
    ret += SharedRandom.Last;

    // 5. Hash player render state
    foreach (var p in Players)
        if (p.UnlockedRenderPlayer)
            ret += Sync.HashPlayer(p);

    return ret;
}
```

Trait hash 用 XOR 組合所有 `[Sync]` 標記的欄位。自定義 hash 函數：
- `HashActor(a)` = `(int)(a.ActorID << 16)`
- `HashPlayer(p)` = `(int)(p.PlayerActor.ActorID << 16) * 0x567`
- `HashInt2(i2)` = `((i2.X * 5) ^ (i2.Y * 3)) / 4`
- `HashCPos(c)` = `c.Bits`
- WPos/WVec/WAngle/WDist = `.GetHashCode()`

### 驗證流程

```
cargo test
  1. 解析 .orarep → 取出 orders + SyncHash per tick
  2. 用 orders 驅動 Rust 模擬引擎
  3. 每 tick 結束後算 World.sync_hash()
  4. 比對 Rust hash vs replay 裡的 hash
  → FAIL: tick 42, expected 605399687, got 605399688
  → 找差異，修 bug，再跑
```

**優勢**：
- 零外部依賴，不需要跑 C# engine
- golden data 已經在 `.orarep` 裡
- `cargo test` 完全 self-contained

**限制**：SyncHash 只告訴你「不一樣」，不告訴你「哪裡不一樣」。Debug 時需要逐模組單元測試定位問題。

---

## 潛在問題

### 確定性風險

| 風險 | 怎麼防 |
|------|--------|
| Activity 轉換時機差 1 tick | 逐行對照 C#，每個 activity 單獨測試 |
| A* tie-breaking | 對照 PathFinder.cs 的 cost 比較邏輯 |
| HashMap 遍歷順序（Rust 隨機，C# 按插入序） | 用 BTreeMap 或 IndexMap |
| 排序穩定性（C# Array.Sort 不穩定） | 用 sort_unstable + 相同 tiebreaker |
| 整數溢位（C# 靜默 wrap，Rust panic） | 用 wrapping_add / wrapping_mul |
| RNG 序列 | ✅ 已逐行對照 MersenneTwister.cs |
| SyncHash 遍歷順序 | 對照 World.Actors 和 ActorsHavingTrait 的迭代順序 |

### 工程風險

| 風險 | 怎麼防 |
|------|--------|
| 不知道 replay 觸發了哪些 Order/Activity | 增量式：碰到不認識的 → skip + 警告 → 補實作 |
| 真實 replay 觸發未列出的邏輯 | 先鎖定最簡單的 replay，再擴展 |
| .oramap 格式比預期複雜 | 先用最簡單的地圖 |
| rules 數值抄錯 | 用腳本從 YAML 自動生成 |

**Scope creep 策略：降級不 crash**。碰到不認識的 Order → skip，不認識的 Activity → unit 變 idle。先讓整場跑完，再補缺失。

---

## 開發計畫（3 Track 並行）

不再需要 Track B（C# golden dump），驗證資料直接從 `.orarep` 的 SyncHash 取得。

```
Track A: openra-data           Track C: openra-sim
 ✅ .orarep parser              ✅ MersenneTwister RNG
 → .oramap parser               → WPos/WAngle/CPos math
 → SHP sprite 解碼              → SyncHash 計算
 → orders.jsonl 匯出            → World tick loop
                                 → 比對 replay SyncHash
        │                              │
        └──────────┐   ┌───────────────┘
                   ▼   ▼
              Track D: openra-wasm
              MVP: 讀預錄 JSON, Canvas2D 彩色方塊
              → WASM + WebGL + sprites
              → 接上即時模擬
```

### Track A：openra-data（檔案解析）— ✅ MVP done

1. ✅ `.orarep` 二進制解析（orders + metadata + SyncHash）
2. → `.oramap` 載入（zip：map.yaml + map.bin + actorslist）
3. → SHP sprite 解碼、Palette 載入
4. → rules 提取腳本（`mods/ra/rules/*.yaml` → Rust 常數）

### Track C：openra-sim（模擬引擎）

1. ✅ MersenneTwister RNG（8 tests pass）
2. → WPos/WAngle/CPos 定點數 math
3. → SyncHash 計算（對照 `Sync.cs` + `World.SyncHash()`）
4. → World tick loop + order dispatch
5. → Move + Attack → SyncHash 比對 pass
6. → Pathfinding、Production、Building、Harvester、Shroud

### Track D：openra-wasm（渲染層）

MVP：讀預錄的 `worldstates.jsonl` → Canvas2D 彩色方塊，可 play/pause。

之後：WASM + WebGL + SHP sprites + 地圖 + 動畫 + Camera + 嵌入 openra-rl.dev。

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

// Training (future): RL agent + bot AI 各自產生 orders
sim.apply_order(agent_order);
sim.apply_order(bot_order);
sim.tick();
let state = sim.snapshot(); // → observation → Python
```

### Training 的前置條件

**對手 AI 是 training 的硬性前提**。三個選項：

| 選項 | 工作量 | 效果 |
|------|--------|------|
| **移植 HackyAI** | 大 | 跟 C# training 行為一致 |
| **簡化 scripted bot** | 中 | 快速可用 |
| **Self-play** | 低 | 兩個 RL agent 對打 |

### Training 額外需要的完整清單

| 工作 | 為什麼 replay 不需要 |
|------|---------------------|
| **對手 AI** | Replay 裡雙方 orders 預錄好 |
| **遊戲初始化** | Replay 由 .orarep + .oramap 決定 |
| **勝負判定** | Replay 跑到最後一個 tick 就停 |
| **完整動作空間**（21 種） | Replay 只需處理出現的 orders |
| **觀測序列化** → Python | Replay 交給 WebGL |
| **Reward 計算** | Replay 不需要 |
| **PyO3 binding** | Replay 用 WASM |
| **128 instance 並行** | Replay 只跑一場 |

消除 JIT crash、gRPC 斷線、128 Docker 容器（~44 GB → ~2.5 GB RAM）。
