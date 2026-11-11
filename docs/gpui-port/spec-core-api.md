# GPUI 移植 · 共享核心 API 盘点

> 目标读者：负责用 GPUI 重写 UI（macOS + Linux）的工程师。
> 本文只做**盘点**，不改任何源码。
>
> **源码基线**：`09a0a202`（`fix(android): follow the clipboard observer's new content payload`），盘点时间 2026-11-10。
> 所有签名均从源码抄录，不做推断；每个结论标注 `文件:行号`。
>
> ⚠️ **工作区是活的**。盘点期间（12:26 → 12:45）检测到并行的未提交改动：
> `crates/manis-pocket-core/Cargo.toml`、`crates/manis-pocket-core/src/lib.rs`、根 `Cargo.toml`，
> 以及新增的 `crates/manis-pocket-app/`（GPUI 壳）。本文主体按 **`09a0a202` 提交**描述核心 API
> （`history.rs` / `model.rs` / `sync.rs` / `platform.rs` 均**未被改动**，API 面不变），
> 涉及被并行改动处会显式标注「工作区已改」。
>
> 相关文档：`docs/gpui-port/spec-macos-ui.md`（另一同事负责，本文不涉及）。

---

## 0. 结论摘要

| 项 | 数量 | 来源 |
|---|---|---|
| `HistoryManager` 公开方法（含构造器） | **33** | `crates/manis-pocket-core/src/history.rs:28-469` |
| 核心数据类型（`uniffi::Record`/`Enum`） | 7 | `crates/manis-pocket-core/src/model.rs` |
| `ClipboardObserver` 回调 | **11** | `crates/manis-pocket-core/src/platform.rs:5-63` |
| `SyncCommand` 变体 | **15** | `crates/manis-pocket-sync/src/state.rs:57-105` |
| `SyncEvent` 变体 | **13** | `crates/manis-pocket-sync/src/types.rs:113-169` |
| `SyncEngine`（core 包装层）方法 | 17（私有+公开） | `crates/manis-pocket-core/src/sync.rs:60-541` |

三句话判断：

1. **核心可以直连，无需 uniffi**。`uniffi::export` 只加在 `impl HistoryManager` 和 `ClipboardObserver` 上，业务逻辑本身是普通 Rust（`history.rs:21-26` 是个普通 struct），GPUI crate 直接 `manis-pocket-core = { path = ... }` 就能调。**唯一例外**：`start_sync` 参数类型是 `Arc<dyn ClipboardObserver>`，这个 trait 带 `#[uniffi::export(with_foreign)]`（`platform.rs:5`），但这不影响 Rust 侧直接实现它。
2. **同步是「回调式 + 后台线程」，不是流**。事件在独立 `std::thread` 里的 tokio runtime 上触发（`sync.rs:231-255`），GPUI 必须自己搭 channel 桥接到 `Entity`。
3. **最大的架构风险是 `load()` 全量加载 + BLOB 无分页**（`storage.rs:288-321`）。因为 Linux 现有实现根本不碰 core，1:1 复刻时这条路径会成为新的性能瓶颈（见 §4）。**工作区里正在写的 GPUI 壳已经在踩这个坑**：`crates/manis-pocket-app/src/main.rs:34-36` 在构造时 `load()` 全量进内存，`main.rs:59` 每次按键再 `all_items.clone()` 传给 `search`，渲染时只能 `.take(60)` 硬截断（`main.rs:70`）。

---

## 1. 现有对外 API 全清单

### 1.1 导出机制（决定 GPUI 怎么依赖）

```rust
// crates/manis-pocket-core/src/lib.rs:1-10
mod error; mod history; mod model; mod platform;
mod search; mod sort; mod storage; mod sync;
uniffi::setup_scaffolding!();
```

| 事实 | 位置 |
|---|---|
| 提交态下所有模块是**私有** `mod`，crate 外无法 `use manis_pocket_core::history::HistoryManager` | `lib.rs:1-8`（提交 `09a0a202`） |
| `pub` 类型只在 uniffi 生成的绑定里可见（`ClipboardItem` 等有 `pub`，但模块私有） | `lib.rs:1-8` + `model.rs:4-62` |
| 提交态 `crate-type = ["cdylib", "staticlib"]`，**没有 `rlib`** | `crates/manis-pocket-core/Cargo.toml:8`（提交 `09a0a202`） |
| **工作区已改**：`crate-type = ["cdylib", "staticlib", "rlib"]`，8 个 `mod` 全部改为 `pub mod` | 未提交 diff；已可 `use manis_pocket_core::history::HistoryManager`（`crates/manis-pocket-app/src/main.rs:14-15` 即为证据） |

> ✅ **这项阻塞在工作区已被并行修复**（盘点期间检测到）。提交态需要自己做，工作区直接可用。
> 若 GPUI 分支要基于 `09a0a202` 重来，记得补这两处：
> `crate-type` 加 `"rlib"` + `mod` 改 `pub mod`（或加 `pub use` 重导出）。
>
> 注意 `manis-pocket-sync` 的 `crate-type = ["lib", "staticlib"]`（`crates/manis-pocket-sync/Cargo.toml:8`）本来就含 `lib`，可直接依赖——所以只有 core 有这个坑。

---

### 1.2 数据类型与字段

#### `ClipboardContent` — 单个内容表示（一条 item 可有多份）

```rust
// crates/manis-pocket-core/src/model.rs:4-10
pub struct ClipboardContent {
    /// UTI 字符串，如 "public.utf8-plain-text"、"public.png"
    pub content_type: String,
    /// 原始字节；可为 None（只有类型没有值）
    pub value: Option<Vec<u8>>,
}
```

| 字段 | 类型 | 语义 | 注意 |
|---|---|---|---|
| `content_type` | `String` | macOS UTI **或** MIME，两种命名混用 | 同步路径会产出 MIME（`sync.rs:471`），本地路径产出 UTI（`Clipboard.swift:226`），UI 侧必须两边都认（`SyncBridge.swift:37-48` 就是在做归一化） |
| `value` | `Option<Vec<u8>>` | 原始字节 | 图片就是 PNG/TIFF 字节，无尺寸/缩略图信息 |

#### `ClipboardItem` — 中心领域模型

```rust
// crates/manis-pocket-core/src/model.rs:13-30
pub struct ClipboardItem {
    pub id: String,
    pub application: Option<String>,      // macOS: bundle id；Linux: 恒为 None
    pub first_copied_at: i64,             // Unix epoch 毫秒
    pub last_copied_at: i64,              // Unix epoch 毫秒
    pub number_of_copies: i32,
    pub pin: Option<String>,              // 固定键字符（"b"、"c"…）
    pub title: String,
    pub contents: Vec<ClipboardContent>,
    pub sync_timestamp: i64,              // Unix epoch 毫秒
    pub sync_source: Option<String>,      // 来源 peer id
    pub sync_deleted: bool,               // 墓碑标记
}
```

| 字段 | UI 用途 | 陷阱 |
|---|---|---|
| `id` | 行标识 / 稳定 key | **去重后不是新的 id**——`add` 命中已有项时返回旧 item（`history.rs:75-99`），UI 不能用 id 判断「新增」 |
| `application` | 应用过滤栏、图标 | Linux 侧无来源应用信息，恒 `None`（`wayland/src/main.rs:526-539` 的 GUI 事件里没有 app 字段） |
| `first/last_copied_at` | 时间分组、排序 | 毫秒；Swift 侧再转 `Date`（`Models/HistoryItem.swift`） |
| `number_of_copies` | 「复制过 N 次」徽标 | i32 |
| `pin` | 固定键 | 值为 `None` 表示未固定；固定逻辑在 core 之外选键（见 §1.4） |
| `title` | 列表主文案 + 搜索目标 | **图片项 title 可能为空**（`Clipboard.swift:288-290` 直接 `return ""`，等 OCR 补）；搜索只搜 title，不搜 contents（`search.rs:44`） |
| `contents` | 粘贴时写回剪贴板 | 二进制大字段，全量加载时占用内存 |
| `sync_timestamp` | 冲突合并 | 只有同步路径维护，本地路径由平台层填 `now`（`Clipboard.swift:248`） |
| `sync_source` | 设备过滤（Swift 的 `excludedDevices`，`History.swift:604-607`） | 本地复制为 `None` |
| `sync_deleted` | 墓碑 | **所有查询都过滤掉它**（`storage.rs:291`、`storage.rs:439`），UI 永远看不到 `true` |

#### `MatchRange` / `SearchResult` / `SearchMode` / `SortBy`

```rust
// crates/manis-pocket-core/src/model.rs:33-37
pub struct MatchRange { pub start: i64, pub end: i64 }   // 半开区间，Unicode scalar 下标

// crates/manis-pocket-core/src/model.rs:40-47
pub struct SearchResult {
    pub item: ClipboardItem,
    pub score: Option<f64>,       // 模糊分，Exact/Regexp 为 None
    pub ranges: Vec<MatchRange>,  // 命中区间，供高亮
}

// crates/manis-pocket-core/src/model.rs:49-62
pub enum SearchMode { Exact, Fuzzy, Regexp, Mixed }
pub enum SortBy { LastCopiedAt, FirstCopiedAt, NumberOfCopies }
```

> ⚠️ **`MatchRange` 是 Unicode scalar 下标**，不是字节、不是 UTF-16、不是字形簇。Swift 侧为此专门写了 scalar→grapheme 的映射（`HistoryItemDecorator.swift:186-195`）。GPUI 里 `SharedString`/`&str` slice 用的是**字节**下标，必须自己写 scalar→byte 映射，否则中文/emoji 标题高亮会 panic 或错位。注释也证实了这一点（`model.rs:32`、`History.swift:187-189`）。

#### `CoreError` — 唯一错误类型

```rust
// crates/manis-pocket-core/src/model.rs:64-77
pub enum CoreError {
    Storage { msg: String },     // SQLite 层
    NotFound { id: String },     // 按 id 找不到
    InvalidArg { msg: String },  // 参数非法（如空 item）
    Sync { msg: String },        // 同步引擎
}
```

| 变体 | 触发点 |
|---|---|
| `Storage` | rusqlite 错误自动转换（`model.rs:79-83`）、Mutex 投毒（`history.rs:51` 等全部 `map_err`） |
| `NotFound` | `toggle_pin`（`history.rs:150`）、`update_item_text`（`history.rs:171`） |
| `InvalidArg` | `add` 收到 title 与 contents 双空（`history.rs:64-68`） |
| `Sync` | 同步引擎锁投毒（`history.rs:263`）、keypair 损坏 / 网络初始化失败（`sync.rs:72-74`、`sync.rs:214-226`） |

`GPUI` 版建议：`CoreError` 直接派生 `Clone`（现在没有，`model.rs:64` 只有 `Debug`）以便在事件里传递；或做一层 `thiserror` 包装。

---

### 1.3 `HistoryManager` 全方法表（33 个）

结构体定义：

```rust
// crates/manis-pocket-core/src/history.rs:21-26
#[derive(uniffi::Object)]
pub struct HistoryManager {
    storage: Mutex<Storage>,                    // rusqlite Connection，一把大锁
    sync_engine: Mutex<Option<SyncEngine>>,
    clipboard_state_path: Option<PathBuf>,
}
```

> **并发模型**：整个 SQLite 连接被 `Mutex` 包住，所有历史操作串行；`sync_engine` 另一把锁，所有 `sync_*` 调用都要拿它。UI 线程和多线程同时调用会互相阻塞，但**不会 UB**。GPUI 建议把所有 core 调用放到 `cx.background_executor()`，避免阻塞渲染。

#### A. 初始化与存储（5）

| # | 方法 | 返回 | 语义 | 错误 | 行 |
|---|---|---|---|---|---|
| 1 | `new(db_path: String)` | `Result<Self, CoreError>` | 打开/创建 SQLite 并跑迁移（5 个版本） | `Storage` | `history.rs:30-43` |
| 2 | `load()` | `Result<Vec<ClipboardItem>, CoreError>` | **全量**取出所有未删项，含全部 BLOB，按 `last_copied_at DESC` | `Storage` | `history.rs:48-53` → `storage.rs:288-321` |
| 3 | `count()` | `Result<i64, CoreError>` | 未删除项数（`WHERE sync_deleted = 0`） | `Storage` | `history.rs:210-215` → `storage.rs:437-444` |
| 4 | `storage_size_bytes(db_path: String)` | `i64` | 读 DB 文件元数据大小 | **无 Result，失败返回 0** | `history.rs:203-208` → `storage.rs:446-448` |
| 5 | `migrate_from_swiftdata(swiftdata_path: String)` | `Result<u64, CoreError>` | 从 SwiftData 的 `Z_HISTORYITEM` 迁移，返回条数 | `Storage` | `history.rs:217-222` → `storage.rs:451-571` |

**细节**：
- `db_path` 传 `""` 或 `":memory:"` 时走内存库，且**不**生成 clipboard-state 侧文件（`history.rs:33-37`）。GPUI 测试可直接用 `:memory:`（`history.rs:534` 的测试就是这样）。
- `storage_size_bytes` 的 `db_path` 参数与实例自己的路径无关，是重复入参——UI 必须自己维护路径字符串（Swift 侧 `History.databasePath`，`History.swift:165-183`）。
- `migrate_from_swiftdata` 是 **macOS 专属历史包袱**，Linux 上用不到；且它需要 `&mut Storage`（`storage.rs:451`），而 `HistoryManager` 拿的是 `Mutex` 的 `DerefMut`——调用时会短暂独占整个存储。

#### B. 历史增删改查（3）

| # | 方法 | 返回 | 语义 | 错误 | 行 |
|---|---|---|---|---|---|
| 6 | `add(item: ClipboardItem, max_size: i32, is_unlimited: bool)` | `Result<ClipboardItem, CoreError>` | 去重合并 + 容量裁剪 + 插入 | `InvalidArg` / `Storage` | `history.rs:57-116` |
| 7 | `delete(id: String)` | `Result<(), CoreError>` | 物理删除（contents + item），**不广播** | `Storage` | `history.rs:118-123` → `storage.rs:372-383` |
| 8 | `update_item_text(id: String, new_text: String)` | `Result<ClipboardItem, CoreError>` | 改写 title 与 `public.utf8-plain-text` 内容 | `NotFound` / `Storage` | `history.rs:160-183` |

**`add` 的真实语义（UI 必须理解，否则列表会错乱）**：

1. `title.is_empty() && contents.is_empty()` → `InvalidArg`「empty item」（`history.rs:64-68`）。**注意：只判 title 和 contents 都空**，所以「有 contents 但 title 空」的图片项是合法的。
2. 去重：调 `find_similar`，忽略 8 种瞬态类型（`com.apple.modified`、`com.manispocket.from-manispocket` 等，`history.rs:477-486`），要求**新项所有非瞬态 content 的 (type,value) 都能在旧项里找到**（`history.rs:497-503`）。
3. 命中重复时：`last_copied_at` 覆盖为新值、`number_of_copies += 1`、补上缺失类型的 content、同类型 content 的值被**覆盖**，然后 `update_item`（`history.rs:76-98`）。返回的是**旧 item**（id 不变）。
4. 未命中：先裁剪再插入。裁剪只在 `!is_unlimited && max_size > 0` 时生效（`history.rs:101-112`）：`count >= max_size` 时算 `excess = count - max_size + 1`，把 `get_all_items()`（已按 last_copied_at DESC）里**未固定**项 `rev().take(excess)` 删掉——即删掉最旧的若干个。固定项永不因容量被删。
5. 返回值可能是旧 item，所以 Swift 侧用 `all.contains(where: { $0.item.id == result.id })` 判断「新增 vs 合并」（`History.swift:271-272`）。

> ⚠️ `add` **不广播**同步（`history.rs:57-116` 全函数无 sync 调用）。广播必须由调用方显式调 `sync_broadcast_item`——Swift 在 `History.swift:283-285` 做，且用 `syncBroadcastToPeers` 标志位抑制回声。

`update_item_text` 细节：只在 item 已有 `public.utf8-plain-text` 时替换该 content 的 value；**若原来没有纯文本 content，则只改 title，不新增 content**（`history.rs:172-180`）。用于 append 模式（`History.swift:229`）。

#### C. 搜索与排序（2）

| # | 方法 | 返回 | 语义 | 错误 | 行 |
|---|---|---|---|---|---|
| 9 | `search(query: &str, items: Vec<ClipboardItem>, mode: SearchMode)` | `Vec<SearchResult>` | 纯函数，**无状态**，对传入的 items 过滤 | 无（内部吞掉正则错误返回空） | `history.rs:185-192` → `search.rs:12-30` |
| 10 | `sort(items: Vec<ClipboardItem>, sort_by: SortBy, pin_to_top: bool)` | `Vec<ClipboardItem>` | 纯函数，先按字段降序，再稳定地把固定项推到顶/底 | 无 | `history.rs:194-201` → `sort.rs:4-33` |

**语义要点**：
- `query` 为空 → 原样返回全部 items（`search.rs:13-22`），不做任何过滤。所以「空搜索」等价于「全量」。
- `Exact` 是把 query `regex::escape` 后大小写不敏感匹配 title（`search.rs:32-61`）；`Regexp` 直接编译 query（`search.rs:98-121`）；`Fuzzy` 用 `SkimMatcherV2`，**只对 title 前 5000 字节**做匹配（`search.rs:7`、`search.rs:70-73`），并按 score 降序（`search.rs:89-93`）；`Mixed` = exact → regexp → fuzzy 依次尝试，第一个非空即返回（`search.rs:123-138`）。
- 只搜 `title`，**不搜 contents**（`search.rs:44`、`search.rs:68`、`search.rs:107`）。全量正文搜索是个缺口，见 §4。
- `sort` 的比较键是降序（`b.x.cmp(&a.x)`，`sort.rs:12-16`），再用稳定排序把 pinned 推到前/后（`sort.rs:19-30`）。
- **两个方法都不碰数据库**，入参是完整 `Vec<ClipboardItem>`。意味着每次搜索都要把全量 item 从 UI 层拷贝进来——GPUI 里这是每次按键一次 `Vec<ClipboardItem>` 克隆。Swift 用 0.2s 节流缓解（`History.swift:103`），GPUI 必须自己再节流。

#### D. 固定（1）

| # | 方法 | 返回 | 语义 | 错误 | 行 |
|---|---|---|---|---|---|
| 11 | `toggle_pin(id: String, available_pins: Vec<String>)` | `Result<ClipboardItem, CoreError>` | 已固定→取消；未固定→取 `available_pins.first()` | `NotFound` / `Storage` | `history.rs:139-158` |

**关键**：**选哪个键是 UI 的责任**，core 只取列表第一个（`history.rs:154`）。Swift 在 `History.availablePins` 里算：全部支持的键集合减去已被占用的（`History.swift:587-590`），还会剔除与预览快捷键冲突的键（`History.swift:578-582`）。GPUI 必须原样复刻这套键位分配逻辑，否则多设备/多项固定会撞键。

#### E. 清空（2）

| # | 方法 | 返回 | 语义 | 错误 | 行 |
|---|---|---|---|---|---|
| 12 | `clear_unpinned()` | `Result<u64, CoreError>` | 删除所有 `pin IS NULL`，返回删除行数 | `Storage` | `history.rs:125-130` → `storage.rs:385-395` |
| 13 | `clear_all()` | `Result<u64, CoreError>` | 清空两张表，返回 item 行数 | `Storage` | `history.rs:132-137` → `storage.rs:397-401` |

> 两者都**只删本地库，不广播删除**。Swift 也没广播（`History.swift:296-317`）——即「清空」不会同步到对端。这是一个已知行为差异，GPUI 若想 1:1 复刻则保持现状，若要修则是产品决策。

#### F. 同步：生命周期（2）

| # | 方法 | 返回 | 语义 | 错误 | 行 |
|---|---|---|---|---|---|
| 14 | `start_sync(device_name: String, device_id: String, observer: Arc<dyn ClipboardObserver>)` | `Result<(), CoreError>` | 启引擎；恢复持久化 keypair 与已配对 peer | `Storage` / `Sync` | `history.rs:227-266` |
| 15 | `stop_sync()` | `Result<(), CoreError>` | 发 `Shutdown` 并 `take()` 掉引擎 | `Sync`（锁投毒） | `history.rs:269-278` |

`start_sync` 内部动作顺序（务必了解，因为 GPUI 要重放）：
1. 从 `sync_config` 表读 `libp2p_keypair`（`history.rs:233-237`）；
2. 从 `sync_pairs` 读已配对 peer id 列表（`history.rs:240-247`）；
3. `SyncEngine::start(...)` 起后台线程（`history.rs:248-255` → `sync.rs:64-271`）；
4. 把（可能新生成的）keypair 写回 `sync_config`（`history.rs:256-259`）；
5. 存进 `self.sync_engine`（`history.rs:260-265`）。

> 重复调用 `start_sync` 会**直接覆盖旧引擎**（`history.rs:264`），旧线程不会收到 `Shutdown`——泄漏一个 libp2p 线程。GPUI 必须自己保证幂等（Swift 用 `isStarted` 标志，`SyncBridge.swift:203-206`）。

#### G. 同步：发现与配对（8）

| # | 方法 | 返回 | 语义 | 行 |
|---|---|---|---|---|
| 16 | `sync_add_peer_address(address: String)` | `()` | 手动加入 `/ip4/…/tcp/…` 地址 | `history.rs:280-286` |
| 17 | `sync_start_discovery()` | `()` | mDNS 开始 | `history.rs:288-294` |
| 18 | `sync_stop_discovery()` | `()` | mDNS 停止 | `history.rs:296-302` |
| 19 | `sync_refresh_discovery()` | `()` | 先 stop 再 start，用于找新设备 | `history.rs:305-312` |
| 20 | `sync_request_pairing(peer_id: String)` | `()` | 发起配对请求 | `history.rs:314-320` |
| 21 | `sync_accept_pairing(peer_id: String, pin: String)` | `()` | 用比较码接受 | `history.rs:322-328` |
| 22 | `sync_reject_pairing(peer_id: String)` | `()` | 拒绝 | `history.rs:330-336` |
| 23 | `sync_unpair(peer_id: String)` | `()` | 解除配对（引擎侧） | `history.rs:338-344` |

**这 8 个方法全部返回 `()`，且全部是「引擎未启动就静默 no-op」**（`if let Some(ref e) = *guard`，行号见上表）。UI 无法知道命令是否真的发出去了。GPUI 需要自己在 UI 侧维护「引擎已启动」状态。

⚠️ `sync_unpair` **只告诉引擎**，不删 `sync_pairs` 表。Swift 观察者在 `on_pairing_complete(success: false)` 时才调 `removePairedPeer`（`SyncBridge.swift:113`）。手动解除配对要两步：`sync_unpair` + `remove_paired_peer`。这是重复实现两次路径的典型。

#### H. 同步：广播与当前剪贴板（6）

| # | 方法 | 返回 | 语义 | 行 |
|---|---|---|---|---|
| 24 | `sync_broadcast_item(item: ClipboardItem)` | `()` | 广播新项（内部过滤成 text/PNG） | `history.rs:347-353` |
| 25 | `sync_observe_local_clipboard(content: Option<ClipboardContent>)` | `()` | 上报本机剪贴板；`None` = 真空 → 清空对端 | `history.rs:359-365` |
| 26 | `sync_confirm_current_clipboard(event_id: String, success: bool)` | `()` | 平台写完剪贴板后回报结果 | `history.rs:368-374` |
| 27 | `sync_should_apply_current_clipboard(event_id: String)` | `bool` | 查询是否应应用该远端事件 | `history.rs:376-386` |
| 28 | `sync_broadcast_deletion(item_id: String)` | `()` | 广播删除 | `history.rs:389-395` |
| 29 | `sync_broadcast_update(item: ClipboardItem)` | `()` | 广播更新 | `history.rs:398-404` |

**`sync_broadcast_item` 的隐藏语义**（`sync.rs:332-339` + `sync.rs:441-450`）：
- 只发**纯文本优先**（`public.utf8-plain-text` / `text/plain` / `text/plain;charset=utf-8`），否则发 **PNG**（`public.png` / `image/png`）。
- 文本为空或超过 `MAX_CLIPBOARD_TEXT_BYTES = 512 KiB`（`register.rs:8`）→ 返回 `None` → **整个广播静默跳过**，不会清空对端。
- 都不匹配（文件、超大图）→ 同样跳过。
- Swift 还额外调 `item.withPortablePNG()` 先把 TIFF/JPEG/HEIC 转 PNG 再广播（`SyncBridge.swift:246-248`、`SyncBridge.swift:342-358`）——**核心不做这件事**，是平台责任。

**`sync_observe_local_clipboard` 的语义契约**（`history.rs:355-365` 的文档注释 + `sync.rs:345-354`）：
- `Some(不可分享内容)`（文件、超大图）→ **必须整个跳过**，不能降级成 `Unavailable`，否则会清空对端剪贴板。
- `None` 才是「剪贴板真的空了」，是唯一会清空对端信号。
- Linux 实现为此专门区分了 `ClipboardRead::Unshareable` 与 `Empty`（`wayland/src/main.rs:54-62`、`wayland/src/main.rs:286-290`）。GPUI 必须保留这个三分状态。

**远端写入的确认握手**（`should_apply` → 写平台 → `confirm`）：
- `sync_should_apply_current_clipboard` 内部用 `oneshot` + `blocking_recv()` 同步等待网络线程回话（`sync.rs:363-376`）。
- 它在**持有 `sync_engine` 锁的情况下**做阻塞等待（`history.rs:377-386`）。
- 实际调用顺序见 Swift 观察者：先 `shouldApply` 判断，再改剪贴板，最后 `confirm`（`SyncBridge.swift:23-63`）。

#### I. 文件传输与对端元数据（4）

| # | 方法 | 返回 | 语义 | 行 |
|---|---|---|---|---|
| 30 | `request_file(peer_id: String, file_path: String)` | `()` | 向对端请求文件分块下载 | `history.rs:409-415` |
| 31 | `get_paired_peers()` | `Vec<String>` | 已配对 peer，每项是 **JSON 字符串** | `history.rs:421-446` |
| 32 | `save_paired_peer(peer_id: String, display_name: String, is_admin: bool)` | `()` | 落库；失败只 `log::error!` | `history.rs:447-456` |
| 33 | `remove_paired_peer(peer_id: String)` | `()` | 删库；失败只 `log::error!` | `history.rs:459-468` |

`get_paired_peers` 返回的每个字符串形如（`history.rs:437-443`）：

```json
{"peerId":"...","displayName":"...","isAdmin":true,"isOnline":false}
```

> `isOnline` 来自内存中的 `connected_peers` 集合（`history.rs:429-436` → `sync.rs:403-405`），不是 DB 列。GPUI 版强烈建议把这里改成结构体（见 §4）。

⚠️ 该方法用 `self.storage.lock().unwrap()`（`history.rs:422`）而不是 `map_err`——**锁投毒会 panic**，与同文件其他方法的风格不一致。UI 里调用需注意。

---

### 1.4 UI 实际用到的 API 分布（Swift 侧证据）

| 使用点 | 调用 | 位置 |
|---|---|---|
| 启动载入 | `load()` + `sort()` | `History.swift:182-186` |
| 新增/去重 | `add(item:maxSize:isUnlimited:)` | `History.swift:246` |
| 增量改文本（append） | `update_item_text` | `History.swift:229` |
| 搜索 | `search(query:items:mode:)` | `History.swift:29, 47, 62` |
| 删除 | `delete(id:)` | `History.swift:327` |
| 清空 | `clear_unpinned()` / `clear_all()` | `History.swift:298, 312` |
| 固定 | `toggle_pin(id:availablePins:)` | `History.swift:542` |
| 容量 | `count()` 未用；用 `Defaults[.size]` | `History.swift:196-201` |
| 同步启停 | `start_sync` / `stop_sync` | `SyncBridge.swift:213-237` |
| 对端列表 | `get_paired_peers()` | `Settings/SyncSettingsPane.swift` |
| 全量 API 面 | uniffi 生成协议 | `ManisPocket/Generated/ManisPocketCore.swift:1236-1344` |

**未被 Swift UI 调用的方法**：`count()`、`storage_size_bytes()`、`migrate_from_swiftdata()`（迁移在别处/一次性）、`sync_start_discovery/stop_discovery/refresh_discovery` 中仅 `refreshDiscovery` 被调（`SyncBridge.swift:322-324`）。GPUI 版可以按最低优先级实现，但为 1:1 保留。

---

## 2. 同步引擎的可驱动面

### 2.1 `SyncCommand` 全枚举（15 个变体）

```rust
// crates/manis-pocket-sync/src/state.rs:57-105
#[derive(Debug)]
pub enum SyncCommand {
    StartDiscovery,
    StopDiscovery,
    RequestPairing { peer_id: String },
    AcceptPairing { peer_id: String, pin: String },
    RejectPairing { peer_id: String },
    BroadcastItem { item_json: String },
    ObserveLocalClipboard { value: ClipboardValue },
    CurrentClipboardApplied { event_id: String, success: bool },
    ShouldApplyCurrentClipboard {
        event_id: String,
        reply: tokio::sync::oneshot::Sender<bool>,   // ← 唯一的请求/响应
    },
    BroadcastDeletion { item_id: String },
    BroadcastUpdate { item_json: String },
    AddPeerAddress { address: String },
    Unpair { peer_id: String },
    /// Send a file chunk to a peer.
    SendFileChunk {
        peer_id: String,
        request_id: String,
        file_path: String,
        offset: u64,
    },
    Shutdown,
}
```

| 变体 | 通道 | 谁发 | 说明 |
|---|---|---|---|
| `StartDiscovery` / `StopDiscovery` | fire-and-forget | `sync_start_discovery` / `sync_stop_discovery`（`core/sync.rs:295-301`）；`start()` 里也会自动发一次（`sync.rs:258`） | — |
| `AddPeerAddress` | fire-and-forget | `sync_add_peer_address`（`core/sync.rs:289-293`） | 手动连接 |
| `RequestPairing` / `AcceptPairing` / `RejectPairing` / `Unpair` | fire-and-forget | 对应 4 个 `sync_*` 方法 | — |
| `BroadcastItem` | fire-and-forget | **core 从不发**（`broadcast_item` 走的是 `ObserveLocalClipboard`，`core/sync.rs:332-339`） | 变体存在但是死路径 |
| `ObserveLocalClipboard` | fire-and-forget | `sync_broadcast_item` / `sync_observe_local_clipboard` | 真正承载剪辑板广播 |
| `CurrentClipboardApplied` | fire-and-forget | `sync_confirm_current_clipboard` | 写回执 |
| `ShouldApplyCurrentClipboard` | **oneshot 请求/响应** | `sync_should_apply_current_clipboard` | 唯一同步等待点 |
| `BroadcastDeletion` / `BroadcastUpdate` | fire-and-forget | `sync_broadcast_deletion` / `sync_broadcast_update` | — |
| `SendFileChunk` | fire-and-forget | `request_file`（内部生成 `request_id`，`core/sync.rs:393`） | — |
| `Shutdown` | fire-and-forget | `stop_sync` | 只能停一次，之后 `command_tx.send` 会失败并 `log::error!`（`core/sync.rs:278-285`） |

**`ClipboardValue`（`ObserveLocalClipboard` 的载荷）**：

```rust
// crates/manis-pocket-sync/src/register.rs:31-46
pub enum ClipboardValue {
    Text(String),
    Binary { mime: String, data: String },  // base64；只接受 image/png
    Unavailable,                            // 唯一会清空对端的值
}
```

配额：`MAX_CLIPBOARD_TEXT_BYTES = 512 * 1024`（`register.rs:8`）、`MAX_CLIPBOARD_IMAGE_BYTES = 10 * 1024 * 1024`（`register.rs:16`）、`SHAREABLE_IMAGE_MIME = "image/png"`（`register.rs:21`）。`is_shareable()` 是权威校验（`register.rs:52-66`）。

### 2.2 `SyncEvent` 全枚举（13 个变体）

```rust
// crates/manis-pocket-sync/src/types.rs:113-169
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum SyncEvent {
    #[serde(rename = "peer_discovered")]  PeerDiscovered { peer: PeerInfo },
    #[serde(rename = "peer_lost")]        PeerLost { peer_id: String },
    #[serde(rename = "pairing_request")]  PairingRequest { peer_id: String, display_name: String, pin: String },
    #[serde(rename = "pairing_complete")] PairingComplete { peer_id: String, success: bool },
    #[serde(rename = "item_received")]    ItemReceived { item_json: String, peer_id: String },
    #[serde(rename = "current_clipboard_received")] CurrentClipboardReceived {
        event_id: String, peer_id: String, content: Option<SyncItemContent>,
    },
    #[serde(rename = "item_deleted")]     ItemDeleted { item_id: String },
    #[serde(rename = "item_updated")]     ItemUpdated { item_json: String },
    #[serde(rename = "error")]            Error { code: i32, message: String },
    #[serde(rename = "listening")]        Listening { address: String },
    #[serde(rename = "file_request")]     FileRequestReceived { peer_id, request_id, file_path, offset: u64 },
    #[serde(rename = "file_chunk")]       FileChunkReceived {
        request_id, file_name, file_size: u64, chunk_index: u32, total_chunks: u32, data: Vec<u8> },
    #[serde(rename = "file_complete")]    FileDownloadComplete { request_id, file_path, success: bool },
}
```

配套：`PeerInfo { peer_id, display_name, addresses: Vec<String>, is_connected }`（`types.rs:5-11`）、`SyncItem`（`types.rs:14-26`）、`SyncItemContent { content_type, value: Option<String> }`（`types.rs:29-34`，`value` 恒为 base64 字符串）。

> **两个变体在 core 里被丢弃**：
> - `FileRequestReceived` → `// Handled by NetworkManager internally`，不进 observer（`core/sync.rs:180-182`）。
> - `ItemReceived` / `ItemUpdated` 的 `item_json` 是 `SyncItem` 的 JSON，core 会 `deserialize_item` 转成 `ClipboardItem` 再交给 observer（`core/sync.rs:92-96`、`142-146`）；**解析失败则静默丢弃**（`deserialize_item` 返回 `Err(())`）。
>
> GPUI 若直接用 `SyncEvent`（绕过 core），需要自己重做 base64 解码与时间戳解析（`core/sync.rs:499-541`）。**结论：应该用 core 的 `SyncEngine`，不要直接用 `SyncEvent`。**

### 2.3 core 如何包装（`SyncEngine`）

```rust
// crates/manis-pocket-core/src/sync.rs:14-21
pub struct SyncEngine {
    state: SharedState,                                   // Arc<Mutex<SyncState>>
    command_tx: mpsc::UnboundedSender<SyncCommand>,
    observer: Arc<dyn ClipboardObserver>,
    connected_peers: Arc<std::sync::Mutex<HashSet<String>>>,  // 只由事件回调更新
}
```

**命令侧映射**（`HistoryManager::sync_*` → `SyncEngine` → `SyncCommand`）：

| core 公开方法 | `SyncEngine` 方法 | `SyncCommand` | 是否有返回值 |
|---|---|---|---|
| `start_sync` | `SyncEngine::start`（关联函数） | 内部发 `StartDiscovery` | `Result` |
| `stop_sync` | `stop` | `Shutdown` | 否 |
| `sync_add_peer_address` | `add_peer_address` | `AddPeerAddress` | 否 |
| `sync_start_discovery` | `start_discovery` | `StartDiscovery` | 否 |
| `sync_stop_discovery` | `stop_discovery` | `StopDiscovery` | 否 |
| `sync_refresh_discovery` | 直接 stop+start | `StopDiscovery` + `StartDiscovery` | 否 |
| `sync_request_pairing` | `request_pairing` | `RequestPairing` | 否 |
| `sync_accept_pairing` | `accept_pairing` | `AcceptPairing` | 否 |
| `sync_reject_pairing` | `reject_pairing` | `RejectPairing` | 否 |
| `sync_unpair` | `unpair` | `Unpair` | 否 |
| `sync_broadcast_item` | `broadcast_item` | `ObserveLocalClipboard` | 否（不可分享则完全不发） |
| `sync_observe_local_clipboard` | `observe_local_clipboard` | `ObserveLocalClipboard` | 否 |
| `sync_confirm_current_clipboard` | `confirm_current_clipboard` | `CurrentClipboardApplied` | 否 |
| `sync_should_apply_current_clipboard` | `should_apply_current_clipboard` | `ShouldApplyCurrentClipboard` | **是（阻塞）** |
| `sync_broadcast_deletion` | `broadcast_deletion` | `BroadcastDeletion` | 否 |
| `sync_broadcast_update` | `broadcast_update` | `BroadcastUpdate` | 否 |
| `request_file` | `request_file` | `SendFileChunk` | 否 |

**事件侧映射**（`SyncEvent` → `ClipboardObserver`，见 `core/sync.rs:89-209`）：11 个 observer 回调里有 10 个是一对一转发，唯一的「加工」发生在两处：

1. `ItemReceived`：把 `sync_source` 填成来源 `peer_id`（`core/sync.rs:93-96`）。
2. `CurrentClipboardReceived`：base64 解码 → 组装 `ClipboardItem`（id 用 `event_id`、时间用 `now`、title 取文本前 120 字符、图片项 title 为空、`sync_source = peer_id`）（`core/sync.rs:98-138`）。

> 这个 `CurrentClipboardReceived → on_item_received` 的降级是**必须知道的行为**：远端剪贴板更新走的是 `on_item_received`，与普通历史项同一条回调。Swift 只能靠「内容为空 vs 非空」区分清空与更新（`SyncBridge.swift:23-30`）。

### 2.4 线程模型（决定 GPUI 怎么接）

```
┌─ UI 线程（Swift MainActor / GPUI 主线程）
│    HistoryManager::sync_*  ──── mpsc::UnboundedSender<SyncCommand> ──┐
│                                                                      │
├─ 后台 std::thread（sync.rs:231 `std::thread::spawn`）                 │
│    tokio::runtime::Runtime::new()（sync.rs:232）                      │
│    rt.block_on(NetworkManager::run())（sync.rs:254）  ← 消费命令 ◄─────┘
│                                                                      │
│    NetworkManager 事件循环（network.rs:236-268）
│      → state_emit(SyncEvent)（network.rs:273-276）
│      → SyncState::emit()（state.rs:40-47）
│      → on_event 闭包（core/sync.rs:89-209）
│      → ClipboardObserver::on_*（**在后台线程执行**）
└──────────────────────────────────────────────────────────────────────┘
```

关键事实与证据：

| # | 事实 | 证据 |
|---|---|---|
| 1 | 引擎跑在**独立的 `std::thread` + 独立 tokio runtime**，与 UI 线程无关 | `core/sync.rs:231-255` |
| 2 | `on_event` 回调**在 network 线程上同步执行**，没有切线程 | `network.rs:273-276` → `state.rs:40-47` |
| 3 | `ClipboardObserver` 要求 `Send + Sync + Debug`，回调是 `&self` 同步方法（无 `async`） | `platform.rs:6-63` |
| 4 | 事件是**推模型**，没有拉取/流式 API | `platform.rs:5` 只有回调 |
| 5 | Swift 用 `Task { @MainActor in }` / `DispatchQueue.main.async` 切回主线程 | `SyncBridge.swift:22, 45, 51, 57` 等 |
| 6 | `SyncState` 自己还建了一个**从未使用**的 tokio Runtime | `state.rs:27` + `wayland/src/main.rs:97-98` 的注释 |
| 7 | `should_apply_current_clipboard` 用 `blocking_recv()` **阻塞当前线程**，且持锁 | `core/sync.rs:363-376` + `history.rs:377-386` |

> ⚠️ **`blocking_recv()` 的硬约束**：tokio 文档明确「在异步执行上下文中调用会 panic」。所以：
> - ❌ 不能在 `tokio::spawn` 里调 `sync_should_apply_current_clipboard`；
> - ❌ 不能在 GPUI 的 foreground executor 里调（不会 panic，但会**卡住 UI 线程**，直到网络线程回话）；
> - ✅ 只能在**普通阻塞线程**里调，GPUI 用 `cx.background_executor().spawn(...)`。
>
> 这就是 §3 里「当前剪贴板应用」必须放在平台/后台层的原因。

### 2.5 GPUI 接入方案（推荐形态）

分三层，**回调只做转发，不碰 UI 状态**：

```rust
// ① 同步事件枚举（GPUI 自己的、可 Clone 的 UI 事件）
#[derive(Clone)]
enum AppEvent {
    ItemReceived(ClipboardItem),
    ItemDeleted(String),
    ItemUpdated(ClipboardItem),
    PeerDiscovered { peer_id: String, name: String, addresses: Vec<String>, connected: bool },
    PeerLost(String),
    PairingRequest { peer_id: String, name: String, pin: String },
    PairingComplete { peer_id: String, success: bool },
    Listening(String),
    SyncError { code: i32, message: String },
    FileChunk { request_id: String, file_name: String, file_size: i64,
                chunk_index: i32, total_chunks: i32, data: Vec<u8> },
    FileDownloadComplete { request_id: String, file_path: String, success: bool },
}

// ② ClipboardObserver 实现：每行只做 send，绝不加锁、不调 core、不碰 UI
struct SyncEventBridge {
    tx: tokio::sync::mpsc::UnboundedSender<AppEvent>,
}

impl ClipboardObserver for SyncEventBridge {
    fn on_item_received(&self, item: ClipboardItem) {
        let _ = self.tx.send(AppEvent::ItemReceived(item));
    }
    fn on_item_deleted(&self, item_id: String) {
        let _ = self.tx.send(AppEvent::ItemDeleted(item_id));
    }
    // …其余 9 个回调同理
}
```

```rust
// ③ 实体侧：把 channel 接到 Entity 上
impl HistoryView {
    fn boot_sync(&mut self, cx: &mut Context<Self>, window: &mut Window) {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<AppEvent>();
        let bridge: Arc<dyn ClipboardObserver> = Arc::new(SyncEventBridge { tx });

        // 注意：start_sync 会 spawn 自己的线程，这里不阻塞
        if let Err(e) = self.core.start_sync(device_name, device_id, bridge) {
            self.status = format!("sync 启动失败: {e:?}");
        }

        // GPUI 里用 spawn_in + WeakEntity，channel 关闭时自然退出
        cx.spawn_in(window, |this, cx| async move {
            while let Some(event) = rx.recv().await {
                // 跨线程 → UI 线程，统一在这里改状态并 notify
                if this.update_in(cx, |this, window, cx| {
                    this.apply_event(event, window, cx);
                    cx.notify();
                }).is_err() {
                    break;   // Entity 已销毁
                }
            }
        })
        .detach();
    }
}
```

要点：

- **`tokio::sync::mpsc` 而不是 `std::sync::mpsc`**：`UnboundedReceiver::recv()` 是纯 waker 驱动的 future，**不需要 tokio runtime**，可以安全地在 GPUI executor 上 `.await`。（`std::sync::mpsc` 的 `recv()` 会阻塞 executor 线程。）
- GPUI crate 需要显式依赖 `tokio = { version = "1", features = ["sync"] }`——core 虽已依赖，但 feature 不传递。
- **不要**在 observer 回调里直接 `cx.notify()`：那要先拿到 `App`/`Window`，而回调没有，且会在 network 线程上做 UI 操作。
- **不要**在回调里调 `core.load()` 或其他 `HistoryManager` 方法：会与 UI 线程抢 `storage` 锁，且在 network 线程上做 SQLite IO。
- 剪贴板写入是**另一条路径**（`should_apply` → 平台写 → `confirm`）。它必须在后台 executor 里串行完成，不能放在 `on_item_received` 里：

```rust
// 远端剪贴板应用：整条链路必须在「阻塞线程」上串行执行
fn apply_remote_clipboard(item: ClipboardItem, core: Arc<HistoryManager>,
                          backend: Arc<dyn ClipboardBackend>, cx: &AsyncApp) {
    cx.background_executor().spawn(async move {
        // ① 问一句「该不该应用」——内部 blocking_recv，绝不能放 UI 线程
        if !core.sync_should_apply_current_clipboard(item.id.clone()) {
            core.sync_confirm_current_clipboard(item.id.clone(), false);
            return;
        }
        if item.contents.is_empty() {
            backend.clear();                                   // 见下方说明②
            core.sync_confirm_current_clipboard(item.id, true);
            return;
        }
        // ② 平台写剪贴板：macOS 侧 AppKit 要求主线程，
        //    由平台实现内部 post 到主线程并同步等待结果，这里看到的是阻塞的 bool
        let ok = backend.write_contents(&item.contents).is_ok();
        core.sync_confirm_current_clipboard(item.id, ok);
    })
    .detach();
}
```

> Swift 的等价实现见 `SyncBridge.swift:23-63`：判断 → 归一化 MIME→UTI → `add()`（用 `syncBroadcastToPeers = false` 抑制回声）→ 写剪贴板 → 回读校验 → `confirm`。
>
> **两条线程约束不要混淆**（很容易写反）：
> 1. `sync_should_apply_current_clipboard` / `sync_confirm_current_clipboard` 是 **core 调用**，必须离开 tokio 与 GPUI foreground executor，放 `background_executor`；
> 2. `ClipboardBackend::write_contents` 是 **AppKit 调用**，必须回到主线程（macOS），因此平台实现内部要再跳一次主线程并同步等待结果。
>
> 也就是说这一小段里必然存在一次「后台 → 主线程 → 后台」的往返。GPUI 里不要在 `render` 或 `update` 的闭包里做②，否则会和①的阻塞互相等待。

---

## 3. 平台差异点清单

### 3.1 macOS 端在 Swift 做的事

| 能力 | macOS 实现 | 证据 |
|---|---|---|
| 剪贴板变更监听 | `NSPasteboard.changeCount` 差量轮询，`Timer` 周期来自 `Defaults[.clipboardCheckInterval]`（默认 **0.5s**） | `Clipboard.swift:11-19, 61-69, 163-171`；`Defaults.Keys+Names.swift:16` |
| 剪贴板读 | `pasteboard.pasteboardItems` → 按 UTI 取 `data(forType:)`，过滤瞬态/动态/Microsoft 前缀类型 | `Clipboard.swift:204-229` |
| 剪贴板写 | `clearContents()` + `setData(_:forType:)`；文件 URL 走 `writeObjects` | `Clipboard.swift:89-115` |
| 防回声标记 | 写剪贴板时打 `com.kaigedong.manispocket` UTI，监听时见到就直接 return | `Clipboard.swift:114, 176-178`；`NSPasteboard.PasteboardType+Types.swift:15` |
| 忽略规则 | 类型白/黑名单 + app bundle id 白/黑名单 + 正则 | `Clipboard.swift:393-423` |
| 粘贴模拟 | `CGEvent` 造 Cmd+V，`post(tap: .cgSessionEventTap)` | `Clipboard.swift:127-147` |
| 权限检查 | `AXIsProcessTrustedWithOptions(nil)`（**只读，不弹窗**） | `Accessibility.swift:4-6` |
| 应用图标 | `NSWorkspace.urlForApplication(withBundleIdentifier:)` + `icon(forFile:)`，进程内缓存 + FSEvent 监听改动 | `ApplicationImage.swift:33-70`、`ApplicationImageCache.swift` |
| 应用标识 | `NSWorkspace.shared.frontmostApplication?.bundleIdentifier` | `Clipboard.swift:45-47, 200, 239-241` |
| OCR | Vision `VNRecognizeTextRequest`（`.fast`）跑在 `CGImage` 上，结果写入 `title` | `Models/HistoryItem.swift:240-267` |
| 标题生成 | 富文本降级（RTF/HTML → 纯文本）、file URL 展开、特殊符号替换、截断 1000 字符 | `Clipboard.swift:287-309, 350-385` |
| 图片格式归一 | `NSImage` → `tiffRepresentation` → `NSBitmapImageRep` → PNG | `SyncBridge.swift:283-289`、`SyncBridge.swift:342-358` |
| 缩略图 | `NSImage.resized(to:)`，异步任务 + 取消 | `HistoryItemDecorator.swift:17, 57-59, 123-177` |
| 全局快捷键 | `KeyboardShortcuts` 库 + Sauce 取键码 | `GlobalHotKey.swift` |
| 通知 / 音效 | `UNUserNotificationCenter` + `NSSound` | `Notifier.swift` |
| 文件下载落盘 | 分块拼装 → `~/Downloads/<name>` | `SyncBridge.swift:392-420` |
| 配置存储 | `Defaults` 库（UserDefaults） | `Extensions/Defaults.Keys+Names.swift` |
| 数据库路径 | `~/Library/Application Support/ManisPocket/manispocket.db`，首次启动从 legacy `Maccy/maccy.db` 复制 | `History.swift:165-183` |

### 3.2 Linux 端在 `manis-pocket-wayland` 做的事

| 能力 | Linux 实现 | 证据 |
|---|---|---|
| 剪贴板监听 | `wl-paste --list-types` / `--type` 子进程，`tokio::time::interval` **350ms** 轮询 | `wayland/src/main.rs:179-180, 368-406` |
| 空选区判定 | 匹配 stderr 的 "Nothing is copied" / "No suitable type" / "No selection" | `wayland/src/main.rs:361-366` |
| 可分享性三分 | `Shareable` / `Unshareable` / 错误，超大 payload 单独处理 | `wayland/src/main.rs:54-62, 428-430, 457-459` |
| 剪贴板写 | `wl-copy --type <mime>` + stdin；清空用 `wl-copy --clear` | `wayland/src/main.rs:542-573` |
| 防轮询回声 | 远端应用后 `suppress_poll_until = now + 1s` | `wayland/src/main.rs:244, 271-275` |
| 协议解码 | base64 解码 + MIME 校验 + 尺寸上限，**先校验后写入** | `wayland/src/main.rs:472-522` |
| 环境校验 | 缺 `WAYLAND_DISPLAY` 直接报错；缺 `wl-paste`/`wl-copy` 报「install wl-clipboard」 | `wayland/src/main.rs:79-87` |
| 配置目录 | `$XDG_CONFIG_HOME/manis-pocket`（0700），`identity.bin`（0600）、`paired-peers.json` | `wayland/src/main.rs:599-647` |
| 密钥持久化 | 自己的 ed25519 keypair 文件，peer_id 由公钥导出 | `wayland/src/main.rs:92-94, 608-617` |
| 对端持久化 | **自己维护 JSON 文件**，与 core 的 `sync_pairs` 表**互不相干** | `wayland/src/main.rs:619-633` |
| GUI 通信 | stdout 打 `@@MANIS_POCKET_EVENT {json}`，stdin 读文本命令 | `wayland/src/main.rs:70-75, 145-157, 306-357` |

### 3.3 GTK4 壳（`manis-pocket-linux`）

**这份代码完全不依赖 `manis-pocket-core`**（`crates/manis-pocket-linux/Cargo.toml` 只有 `gtk4` + `serde_json`）：

| 能力 | 现状 | 证据 |
|---|---|---|
| 后端启动 | `Command::new(current_exe().with_file_name("manis-pocket-wayland"))` 起子进程，`--gui-protocol` | `linux/src/main.rs:549-553` |
| 历史存储 | **纯文本 `Vec<String>`**，存 `~/.local/share/manis-pocket/history.json`，上限 `HISTORY_LIMIT = 100` | `linux/src/main.rs:14, 1124-1157` |
| 图片历史 | **完全不支持** | `history: Vec<String>`（`linux/src/main.rs:38`） |
| 搜索 | 无（只有 GTK 行过滤/highlight） | `linux/src/main.rs:132-164, 884` |
| 剪贴板写 | 自己调 `wl-copy` | `linux/src/main.rs:1107-1122` |
| 应用图标 / 应用过滤 | 无 | 全文无 icon/pixbuf/app 相关 |
| 全局快捷键 | 无 | 全文无 shortcut/accelerator |
| 通知 | 无（依赖只有 gtk4） | `linux/Cargo.toml` |

> **这条差异是移植里最重要的一条**：Linux 现在的「历史」是一个 100 条的字符串数组，既没有 SQLite、也没有 item 模型、也没有同步历史。1:1 复刻意味着 Linux 要**第一次真正接上 `HistoryManager`**。

### 3.4 平台差异表（GPUI 分层建议）

| 能力 | macOS 实现 | Linux 实现 | GPUI 版应放哪一层 |
|---|---|---|---|
| 剪贴板变更监听 | `changeCount` 轮询 0.5s（`Clipboard.swift:61-69`） | `wl-paste` 轮询 350ms（`wayland/src/main.rs:179`） | **`trait ClipboardBackend`**（平台 crate 实现）+ 轮询器在后台 executor；建议统一到 300–500ms 可配 |
| 剪贴板读（多类型） | NSPasteboard 多 item 多 UTI（`Clipboard.swift:204-229`） | 只有 text/plain 与 image/png（`wayland/src/main.rs:395-406`） | `trait ClipboardBackend::read() -> ClipboardRead`，其中 `ClipboardRead` 保留 **Empty / Unshareable / Payload** 三分（对齐 `wayland/src/main.rs:59-62`） |
| 剪贴板写（含 MIME↔UTI 归一） | `setData` + 文件 URL 特例（`Clipboard.swift:96-112`） | `wl-copy --type`（`wayland/src/main.rs:561-573`） | `trait ClipboardBackend::write(&[ClipboardContent])`；**MIME↔UTI 映射写在平台实现内**，不进 core |
| 防回声标记 | 自定义 UTI `com.kaigedong.manispocket`（`Clipboard.swift:114, 176-178`） | 时间窗抑制 1s（`wayland/src/main.rs:244`） | 平台实现内部细节；**core 无此概念**，不要在共享层做 |
| 写后校验 | 回读比对（`SyncBridge.swift:55-62`） | 无 | `trait ClipboardBackend::verify_written()`；GPUI 需补 Linux 侧（`wl-paste` 回读） |
| 粘贴模拟 | `CGEvent` Cmd+V（`Clipboard.swift:127-147`） | **无** | `trait PasteSimulator`；macOS 必需（用户黏贴是主交互），**Linux 是明确缺口**（需 uinput/ydotool/portal，见 §4） |
| 辅助功能权限 | `AXIsProcessTrustedWithOptions`（`Accessibility.swift`） | 无对应概念 | `trait Permissions`，仅 macOS 实现；GPUI 需自己补「未授权时引导」 |
| 应用图标 | `NSWorkspace` + 缓存 + FSEvent（`ApplicationImage.swift`） | **无** | `trait AppIconProvider::icon(bundle_id) -> Option<ImageData>`；缓存放 UI 层 |
| 来源应用标识 | `frontmostApplication.bundleIdentifier`（`Clipboard.swift:45-47`） | **无** | `ClipboardRead` 里带 `application: Option<String>`；Linux 恒 `None`，UI 必须能降级（隐藏应用过滤栏） |
| OCR | Vision（`HistoryItem.swift:240-267`） | **无** | `trait OcrProvider`；仅 macOS 实现，Linux 返回 `None` |
| 富文本 → 标题 | `NSAttributedString(rtf/html)`（`Clipboard.swift:368-382`） | **无** | 平台层或共享 helper crate（可用 `rtf-parser` + `html2text`），**不要塞进 core** |
| 图片格式 → PNG | `NSImage` + `NSBitmapImageRep`（`SyncBridge.swift:283-289`） | 直接就是 PNG | 需要引入 `image` crate；macOS 的 TIFF/HEIC 必须转换（HEIC 需 `libheif` 或走系统 API） |
| 缩略图 | `NSImage.resized`（`HistoryItemDecorator.swift:163-171`） | **无** | UI 层（GPUI 有 `img()` element 与 `RenderImage`）；需要 `image` crate 生成 |
| 全局快捷键 | `KeyboardShortcuts` + Sauce（`GlobalHotKey.swift`） | **无** | `trait GlobalHotkey`；macOS 用 Carbon `RegisterEventHotKey`，Linux 用 X11 grab 或 `org.freedesktop.portal.GlobalShortcuts` |
| 通知 / 音效 | `UNUserNotificationCenter` + `NSSound`（`Notifier.swift`） | **无** | `trait Notifier`；Linux 用 `notify-rust`（libnotify/DBus） |
| 文件下载 | `FileDownloadManager` → `~/Downloads`（`SyncBridge.swift:392-420`） | **无** | 建议**上移到 core**（纯 IO，无平台依赖），GPUI 只显示进度 |
| 剪贴板图片历史 | 支持（BLOB） | **不支持** | 统一到 `HistoryManager`；GPUI 必须做二进制渲染与懒缩略图 |
| 历史存储 | core SQLite（`History.swift:165-183`） | JSON 字符串数组（`linux/src/main.rs:1124-1157`） | **统一到 core `HistoryManager`**；Linux 需要一次性迁移脚本（把 `history.json` 导入 SQLite） |
| 同步对端持久化 | core `sync_pairs` 表（`storage.rs:172-236`） | 自己的 `paired-peers.json`（`wayland/src/main.rs:619-633`） | **统一到 core**；Linux 需迁移 |
| 配置项 | `Defaults`（UserDefaults） | 环境变量 + 硬编码 | 新建 `trait SettingsStore` 或用 `serde` + `directories` crate，**不要复刻 UserDefaults** |
| 忽略规则 | 类型/正则/app 三类（`Clipboard.swift:393-423`） | **无** | 上移到 core（纯逻辑）或共享 crate；目前是 macOS 独有 |
| DB 路径 | `~/Library/Application Support/…` | `~/.config/…` | `directories` crate 按平台给路径 |

---

## 4. 缺失的 API（1:1 复刻所需，本文只列不改）

按「对 GPUI 移植的阻塞程度」排序。

### P0 — 不做就无法启动

#### G1. `crate-type` 缺 `rlib`，模块私有 ✅ 工作区已修

- **提交态**：`crate-type = ["cdylib", "staticlib"]`（`Cargo.toml:8`），`mod history` 等全部私有（`lib.rs:1-8`）。
- **影响（若不改）**：GPUI 无法 `use manis_pocket_core::...`。
- **工作区现状**：已改为 `["cdylib", "staticlib", "rlib"]` + 8 个 `pub mod`（未提交）；`crates/manis-pocket-app/src/main.rs:14-15` 已经在这么用了。
- **补齐方式**（基于提交重建时）：加 `"rlib"`，`mod` 改 `pub mod`（或 `pub use history::HistoryManager;`）。uniffi 导出不受影响。
- **剩余风险**：`pub mod` 把 `storage`、`sort`、`search` 内部实现也暴露成了公开 API 表面，后续重构会被外部依赖锁死。**建议只 `pub use` 需要的那几个类型**，其余保持私有。

#### G1b. macOS 数据库文件名不一致，历史不会「自动继承」

- **现状**：
  - Swift 版：`~/Library/Application Support/ManisPocket/manispocket.db`（`ManisPocket/Observables/History.swift:159-160`）。
  - 工作区 GPUI 壳：`dirs::data_dir()` + `ManisPocket/` 目录，但文件名是 **`Storage.sqlite`**（`crates/manis-pocket-app/src/main.rs:164-169`），且注释写着 "Same location the SwiftUI app uses, so history carries over"。
- **影响**：目录一致、**文件名不一致**，所以迁移过来的用户打开 GPUI 版会看到**空历史**，而旧库仍躺在旁边。这与注释的意图相反，属于会「静默丢数据观感」的 P0 级缺陷。
- **建议**：要么复用 `manispocket.db` 文件名，要么在首次启动做一次 `manispocket.db → Storage.sqlite` 的探测+复制（Swift 侧对 legacy `Maccy/maccy.db` 就是这么干的，`History.swift:162-176`），并在 UI 上提示迁移条数（`migrate_from_swiftdata` 只能迁 SwiftData，**不能迁这个 SQLite 库**——两边 schema 不同，见 G1c）。

#### G1c. 缺少「SQLite → SQLite」的历史库迁移

- **现状**：`migrate_from_swiftdata` 只读 SwiftData 的 `Z_HISTORYITEM` / `Z_HISTORYITEMCONTENT`（`storage.rs:451-571`），是 Core Data 表结构；对「Manis Pocket 自己的 SQLite」没有迁移入口。
- **影响**：若 GPUI 版换了库文件名（G1b）或改了 schema，没有可复用的搬迁路径。Linux 侧要迁移 `~/.local/share/manis-pocket/history.json`（`linux/src/main.rs:1124-1157`，纯字符串数组）同样没有入口。
- **建议接口**：`import_from_own_sqlite(path) -> Result<u64>` + `import_from_text_lines(path) -> Result<u64>`；或统一成一个 `import(path, kind)`。

#### G2. 分页 / 窗口化查询

- **现状**：`load()` 返回**全部**未删项，且每项都带完整 `contents` BLOB（`storage.rs:288-321`）；无 `LIMIT`/`OFFSET`，无「只取元数据」的接口。SQL 里虽有 `idx_items_last_copied`（`storage.rs:98`），但没被用来分页。
- **影响**：
  - 首帧要把整库（含 10 MiB 级图片）反序列化进内存；
  - 每次 `search` 都要把整份 `Vec<ClipboardItem>` 拷进 core（`history.rs:185-192`）；
  - 每次 `sort` 再拷回来（`history.rs:194-201`）。
  - Linux 首次启用 core 后，历史从 100 条字符串变成无上限 BLOB，**这是最大的架构风险**。
- **建议接口**：`load_page(offset: u32, limit: u32) -> Vec<ClipboardItem>`（不含 contents）+ `get_contents(id) -> Vec<ClipboardContent>` 懒加载；或 `load_metadata() -> Vec<ItemMeta>`。

#### G3. 变更事件订阅（历史写操作的通知）

- **现状**：`history.rs` 里**没有任何变更通知**。`add`/`delete`/`clear_*`/`toggle_pin`/`update_item_text` 全是静默写库。唯一的回调通道是同步引擎的 `ClipboardObserver`（`platform.rs:5`），只覆盖「来自对端」的变更。
- **影响**：UI 必须自己作为唯一写入者来维持一致性。GPUI 里一旦有多处写入（本地轮询、同步观察者、菜单命令），就会出现「一处写了、别处不知道」。Swift 目前就是靠 `History.shared` 单例 + `@Observable` 硬扛（`History.swift:10, 106`），这在 GPUI 的 `Entity` 模型里更难维持。
- **建议接口**：`subscribe(listener: Arc<dyn HistoryObserver>)`，或让 `add`/`delete` 等返回一个 `ChangeSet`，由 UI 层统一广播。

### P1 — 复刻会明显走形

#### G4. 按应用 / 按来源设备过滤的查询

- **现状**：无。`application` 只在数据里（`model.rs:16`），过滤完全在 Swift 客户端做（`History.swift:594-611`）。设备过滤同样（`History.swift:604-607`）。
- **影响**：GPUI 要在 UI 层重写一遍过滤 + 与 `search` 的顺序关系（Swift 先过滤再搜索，`History.swift:29`）。应用列表还得自己扫描全量 item 去重（`History.swift:79-99`）——每次刷新 O(n)。
- **建议接口**：`search(query, mode, filter: ItemFilter { application: Option<String>, exclude_sources: Vec<String> })`；外加 `distinct_applications() -> Vec<AppStat>`。

#### G5. 剪贴板写入的原子性 / 事务接口

- **现状**：写剪贴板完全在平台层（`Clipboard.swift:77-125`、`wayland/src/main.rs:542-573`），core 只负责「应该写吗」「写成功了吗」两个问题（`history.rs:368-386`）。失败没有回滚，`confirm(false)` 之后对端会重试（由网络层决定）。
- **影响**：macOS 与 Linux 会写出两套语义略有差异的实现；GPUI 要想 1:1，必须再实现第三套。且「清空 → 写入」不是原子的：`clearContents()` 之后若 `setData` 失败，用户剪贴板就丢了（现在 Swift 没检查失败）。
- **建议接口**：`apply_remote_clipboard(item, backend) -> ApplyOutcome`，把「should_apply → 写 → verify → confirm」四步收进一个 core 函数，平台只提供 `write` 原语。

#### G6. 图片缩略图生成

- **现状**：core 无 `image` 依赖（`crates/manis-pocket-core/Cargo.toml` 依赖列表里没有 `image`）。缩略图在 Swift 用 `NSImage.resized`（`HistoryItemDecorator.swift:163-171`）。
- **影响**：Linux 完全没有等价物；GPUI 若在渲染时对 10 MiB PNG 做解码会掉帧。列表滚动需要预生成缩略图 + 缓存。
- **建议接口**：`thumbnail(id, max_edge: u32) -> Option<Vec<u8>>`（返回编码后的小图），带 LRU 缓存；或至少在 UI 层用 `image` crate + 后台 executor 生成。

#### G7. 标题生成（`generateTitle`）在 Swift 里

- **现状**：`generateTitle`（`Clipboard.swift:287-309`）与 `getPreviewableText`（`Clipboard.swift:350-385`）都在 Swift，依赖 AppKit 解析 RTF/HTML。Linux 完全没有。
- **影响**：`title` 是搜索的唯一目标（`search.rs:44`）。GPUI 若各自实现，macOS 与 Linux 的搜索结果就会不一致；迁移后旧数据的 title 与新数据的 title 规则也会不同。
- **建议接口**：把可移植部分（截断、特殊符号、纯文本/PNG 判定、file URL 展开）上移为 core 的 `build_title(contents) -> String`；RTF/HTML 解析留在平台层注入。

### P2 — 功能缺口 / 瑕疵

#### G8. `promote_to_admin` 未暴露

- **现状**：`Storage::promote_to_admin` 存在（`storage.rs:210-218`），但 `HistoryManager` **没有任何方法调用它**。`history.rs:18` 的文档注释却承诺「Admin can transfer admin role to any member (promote_to_admin)」。
- **影响**：UI 无法实现「转移管理员」。GPUI 若照文档做会找不到接口。
- **附带问题**：`Storage::am_admin()` 的实现是 `count == 0 || true`（`storage.rs:220-226`），恒为 `true`——这是个死逻辑 bug，目前无人调用所以未暴露。

#### G9. `get_paired_peers()` 返回 JSON 字符串

- **现状**：`Vec<String>`，每项 JSON（`history.rs:421-446`）。
- **影响**：跨语言方便，但 Rust UI 里要二次 `serde_json::from_str`，还得处理 `unwrap_or_default()` 产出的空串（`history.rs:443` —— `to_string` 失败会返回空字符串混在列表里）。GPUI 应改成 `Vec<PairedPeer>` 结构体。
- **附带**：`isOnline` 是快照（来自 `connected_peers`，`history.rs:429-436`），没有「在线状态变化」的独立事件，UI 只能等 `on_peer_discovered`/`on_peer_lost` 再重新拉取。

#### G10. 错误表面缺失

- **现状**：`save_paired_peer` / `remove_paired_peer`（`history.rs:447-468`）、全部 `sync_*`（`history.rs:280-415`）返回 `()`；失败只 `log::error!`。
- **影响**：UI 无法提示「配对保存失败」「命令没发出去」。GPUI 里只能靠状态轮询猜。

#### G11. `sync_deleted` 墓碑不可见

- **现状**：所有查询都 `WHERE sync_deleted = 0`（`storage.rs:291`、`storage.rs:439`），且 `deserialize_item` 恒把 `sync_deleted` 设为 `false`（`core/sync.rs:527`）。
- **影响**：无法做「已删除项」的回收站 UI；也无法在删除后与对端做墓碑比对（当前删除靠 `BroadcastDeletion` 实时广播，离线期间删除会丢）。

#### G12. 搜索不覆盖正文

- **现状**：只匹配 `title`（`search.rs:44, 68, 107`）。
- **影响**：图片项 title 来自 OCR，若 OCR 未完成则永远搜不到；多行文本被 `shortened(to: 1000)` 截断后，1000 字符以外的内容不可搜。

#### G13. 权限请求（而非权限查询）

- **现状**：`Accessibility.check()` 调 `AXIsProcessTrustedWithOptions(nil)`——**只读状态，不弹系统授权框**（`Accessibility.swift:4-12`）。
- **影响**：GPUI 需要自己实现「未授权时提示 + 打开系统设置」。Linux 侧没有对应概念，粘贴模拟本身也是缺口。

#### G14. 停用 / 释放的语义

- **现状**：`HistoryManager` 无 `Drop` 实现；`start_sync` 覆盖旧引擎不 stop（见 §1.3 F 组）；`SyncState` 内建一个未使用的 `Runtime`（`state.rs:27`），从异步上下文 drop 它会 panic。
- **影响**：GPUI 里多次开关同步会累积线程；退出时必须显式 `stop_sync()` 且不能从 tokio 上下文里 drop。

---

## 5. 移植风险排序（给 Lead 的决策输入）

| 级别 | 风险 | 依据 | 缓解 |
|---|---|---|---|
| 🔴 P0 | `load()` 全量 + BLOB 无分页，Linux 首次接管后内存/首帧爆炸 | `storage.rs:288-321`；现状已见 `app/src/main.rs:34-36, 59, 70` | 先做 G2，再动 UI |
| 🔴 P0 | 无历史变更订阅，多写入源一致性靠 UI 自觉 | 全文无 observer | 先做 G3，否则 GPUI 实体状态必然漂移 |
| 🔴 P0 | macOS 库文件名 `manispocket.db` → `Storage.sqlite` 不一致，老用户历史「消失」 | `History.swift:159-160` vs `app/src/main.rs:164-169` | 见 G1b：统一文件名或做一次性迁移 |
| 🟠 P1 | 同步回调在 network 线程 + `blocking_recv` 持锁阻塞 | `sync.rs:231-255`、`sync.rs:363-376`、`history.rs:377-386` | §2.5 的 channel + background_executor 方案；禁止在 UI/async 上下文调 `should_apply` |
| 🟠 P1 | Linux 后端与 core 完全脱节（JSON 数组 vs SQLite、独立配对文件） | `linux/src/main.rs:1124-1157`、`wayland/src/main.rs:619-633` | 规划一次性迁移；不要保留两套持久化 |
| 🟠 P1 | 粘贴模拟在 Linux 无解 | `Clipboard.swift:127-147` vs wayland 全文无 | 产品决策：Linux 首版是否接受「只复制不粘贴」 |
| 🟠 P1 | 同步引擎尚未接入 GPUI 壳（当前 0 行同步代码） | `app/src/main.rs` 全文无 `start_sync` | 按 §2.5 一次性把 channel 骨架搭好再补功能 |
| 🟡 P2 | `MatchRange` 是 scalar 下标，GPUI 文本切片是字节下标 | `model.rs:32-37`、`HistoryItemDecorator.swift:186-195` | 写 scalar↔byte 映射工具 + 单测 |
| 🟡 P2 | `title` 生成规则分裂在 Swift，Linux 无 | `Clipboard.swift:287-309` | 上移可移植部分（G7） |
| 🟡 P2 | `pub mod` 一次性暴露 `storage`/`sort`/`search` 内部实现 | 工作区 `lib.rs` diff | 收敛为 `pub use` 白名单 |

---

## 附录 A：来源文件索引

| 文件 | 行数 | 本文引用 |
|---|---|---|
| `crates/manis-pocket-core/src/lib.rs` | 10 | 模块可见性 |
| `crates/manis-pocket-core/src/model.rs` | 83 | 全部数据类型 + `CoreError` |
| `crates/manis-pocket-core/src/history.rs` | 605 | 33 个公开方法、去重、容量裁剪 |
| `crates/manis-pocket-core/src/platform.rs` | 63 | `ClipboardObserver` 11 回调 |
| `crates/manis-pocket-core/src/search.rs` | 272 | 4 种搜索模式 |
| `crates/manis-pocket-core/src/sort.rs` | 107 | 排序 + 固定置顶 |
| `crates/manis-pocket-core/src/storage.rs` | 572 | SQLite schema、迁移、查询 |
| `crates/manis-pocket-core/src/sync.rs` | 542 | `SyncEngine` 包装、线程、序列化 |
| `crates/manis-pocket-sync/src/state.rs` | 105 | `SyncCommand` 15 变体、`SyncState::emit` |
| `crates/manis-pocket-sync/src/types.rs` | 236 | `SyncEvent` 13 变体、`PeerInfo`、常量 |
| `crates/manis-pocket-sync/src/register.rs` | 444 | `ClipboardValue`、限额常量 |
| `crates/manis-pocket-sync/src/network.rs` | 1183 | `NetworkManager::run`、事件发射点 |
| `crates/manis-pocket-wayland/src/main.rs` | 745 | Linux 剪贴板后端全部实现 |
| `crates/manis-pocket-linux/src/main.rs` | 1157 | GTK4 壳、独立持久化 |
| `ManisPocket/Clipboard.swift` | 476 | macOS 剪贴板监听/写入/粘贴 |
| `ManisPocket/Sync/SyncBridge.swift` | 374 | `ClipboardObserver` 的 Swift 实现（线程模型参照） |
| `ManisPocket/Observables/History.swift` | 670 | core API 的实际调用方式 |
| `ManisPocket/Generated/ManisPocketCore.swift` | 2747 | uniffi 生成的完整对外协议 |
| `crates/manis-pocket-app/src/main.rs` | 190 | **工作区新增**的 GPUI 壳（未提交），本文用它校准落地形态 |

## 附录 B：crate 结构 —— 已有现状 vs 建议

**工作区现状**（盘点时检测到，未提交）：

```
crates/
  manis-pocket-core/        # 已加 "rlib" + pub mod（改动未提交）
  manis-pocket-sync/        # 未改
  manis-pocket-wayland/     # 未改，Linux 剪贴板后端
  manis-pocket-linux/       # 未改，GTK4 壳（将退役）
  manis-pocket-app/         # 新增，GPUI 壳，依赖 gpui-kit 0.9.1
    Cargo.toml              #   gpui-kit / manis-pocket-core / dirs / chrono / log
    src/main.rs             #   190 行：单 Entity HistoryPanel + search 列表
```

`manis-pocket-app` 当前已实现的部分（`src/main.rs`）：窗口 450×800（`main.rs:20-21`，对齐 macOS 面板）、输入框 + 列表 + 底部状态条（`main.rs:66-134`）、`load()` + `search()` 驱动（`main.rs:34-36` / 刷新在 `main.rs:57-61`）、`placeholder_title` 兜底图片项（`main.rs:138-145`）、`meta_line` 显示应用名与时间（`main.rs:147-158`）、`storage_path`（`main.rs:161-170`）。

**尚未接入**：同步（`start_sync`/观察者）、剪贴板后端（读/写/轮询）、粘贴模拟、图片渲染与缩略图、分页与虚拟滚动、固定/删除/清空、应用过滤栏、设置持久化。即 §3 与 §4 里除「持久化」外的绝大多数条目都还是空白。

**建议补齐的结构**（在 `manis-pocket-app` 基础上演化，不必新建 crate）：

```
crates/
  manis-pocket-core/            # 现有：加 Profile/Page/Observer（§4 G2/G3）
  manis-pocket-sync/            # 现有：不动
  manis-pocket-platform/        # 新增：trait 定义（无平台依赖）
    src/lib.rs                  #   ClipboardBackend, PasteSimulator, AppIconProvider,
                                #   OcrProvider, GlobalHotkey, Notifier, Permissions,
                                #   SettingsStore；含 ClipboardRead 三分枚举
  manis-pocket-platform-macos/  # objc2 / AppKit 实现
  manis-pocket-platform-linux/  # wl-clipboard 子进程实现（吸收现有 wayland crate 的读/写逻辑）
  manis-pocket-app/             # 现有 GPUI 壳，拆分为：
    src/main.rs                 #   窗口与生命周期
    src/history_view.rs         #   HistoryPanel Entity（已在）
    src/sync_bridge.rs          #   SyncEventBridge（§2.5，待补）
    src/clipboard_pump.rs       #   轮询 → ClipboardRead → 入队（待补）
```

> Linux 侧 `manis-pocket-wayland` 与 `manis-pocket-linux` 最终应退役：前者被 `manis-pocket-platform-linux` 吸收，后者被 `manis-pocket-app` 取代。但**不要一次删**——先让 GPUI 版在两边跑通，再删旧壳。
