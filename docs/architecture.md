# 架构

## 三个 crate

| crate | 职责 | 依赖 |
|---|---|---|
| `manis-pocket-core` | 数据与规则：SQLite 存储、迁移、搜索、排序、去重、同步协议的数据结构 | rusqlite |
| `manis-pocket-sync` | 网络：libp2p（mDNS 发现、gossipsub、request-response）、配对、C 边界 | core |
| `manis-pocket-app` | 界面：GPUI 窗口、面板、设置、托盘、剪贴板读取 | core + sync |

`core` 不依赖 UI，所以数据层可以在没有显示器的情况下测试；`app` 里凡是能脱离窗口的逻辑（行的文本、徽章、搜索高亮、行序、裁剪规则）也都在 `rows.rs` 这类纯函数里。

## 线程与实体

| 谁 | 在哪 | 做什么 |
|---|---|---|
| GPUI 主线程 | 应用生命周期 | 所有窗口、渲染、事件 |
| 剪贴板监听 | 独立线程（macOS 轮询 pasteboard，Linux 走 `wl-paste`） | 变化时把 `Copy` 推进 channel |
| 同步引擎 | 独立 Tokio runtime | libp2p swarm、配对、收发 |
| 控制 socket | 独立线程 | 接收 `--toggle` / `pair` / `accept` |

跨线程只走 channel；剪贴板监听和同步引擎**从不直接碰 GPUI 状态**，它们把事件推给界面，界面在自己的 tick 里取。

## 一次复制发生了什么

```
系统剪贴板变化
  → clipboard::spawn 读到 Copy { Text | Image | File }
    → record_clipboard_change
        · 用当前设置判断是否保存（每次复制都重读设置，不是启动时快照）
        · 忽略规则（应用 / 正则 / 白名单模式）
        · 去重：同一 payload 只更新复制次数和时间
      → 写入 SQLite
      → 广播给已配对设备（文本与 PNG，单条上限 20 MiB）
      → 通知面板：HistoryChanged → 面板重载列表
```

反过来，收到对端条目时：写库 → 把内容写进本机剪贴板（`apply_remote`）→ 推 `HistoryChanged` → 面板重载。

写剪贴板这一步要**回报结果**给引擎：协议在对端确认"已落到剪贴板"之前会一直重试，不回报就会在第一条之后卡死。而本机刚复制过的东西不会被对端覆盖（`should_apply_current_clipboard`）。

## GPUI 的一条硬规则（踩过四次）

**不要在列表自己的更新里读同一个列表实体。**

```rust
// 会 abort：cannot read ListState while it is already being updated
list.update(cx, |state, cx| {
    state.set_selected_index(.., cx);   // 委托回调 → 面板读 list → 崩
});
```

这条规则来自 GPUI 的实体借用模型（类似 `RefCell`），不是内存安全 bug。因此：

- 渲染只读**面板自己缓存的**状态（`visible_count`、`selected_id`、`all_items`），不读 `ListState`
- 选中变化由委托通过 `cx.spawn` 的任务推给面板（任务在帧之间跑）
- `defer` / `defer_in` **不安全**：它们在同一 effect cycle 末尾执行，那时列表还在更新栈上
- 行内按钮（删除、固定）用 `cx.on_next_frame` 把工作推到下一帧

## 平台的差异在哪

| 能力 | macOS | Linux |
|---|---|---|
| 应用图标 | AppKit `NSWorkspace`（1024px → 64px 两级缩放） | 图标主题 + `.desktop` |
| 托盘 | `tray-icon`（NSStatusItem，可热替换图标） | `ksni`（D-Bus StatusNotifier） |
| 粘贴 | `osascript` 合成 ⌘V | `wtype` 合成 Ctrl+V |
| 剪贴板 | `NSPasteboard` 轮询 | `wl-paste`（后台进程没有键盘焦点，只能走它） |

其余全部共享：面板、设置窗口、行渲染、搜索、同步、快捷键。

## 数据存放

| 平台 | 路径 |
|---|---|
| macOS | `~/Library/Application Support/ManisPocket/manispocket.db` |
| Linux | `$XDG_DATA_HOME/manis-pocket/manispocket.db` |

同目录还有 `settings.json` 和控制 socket；旧版本的历史（SwiftUI 的 SwiftData、GTK 的 `history.json`）在首次启动、新库为空时导入一次。
