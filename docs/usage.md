# 使用

- 在 macOS 上按 `⇧⌘C` 打开历史记录，输入关键词搜索。
- 按 `Return` 复制所选内容；按 `⌥Return` 复制并粘贴。
- 按 `⌥P` 固定或取消固定所选项目。
- 在设置中管理历史记录、快捷键和设备同步。

## macOS ↔ Linux Wayland 直接粘贴

Linux 端和 macOS 端是同一个 GPUI 程序（Rust），不依赖 Tauri / Electron / GTK。Arch Linux 安装 `manis-pocket-bin` 后，从应用菜单打开 **Manis Pocket**，也可以在 Wayland 会话中运行 `manis-pocket`。窗口的 **History** 可搜索和重新复制文本，**Devices** 可发现、连接和配对设备。关闭窗口后同步仍在后台运行；重新从应用菜单打开窗口，点击 **Quit** 才会退出。

Linux 窗口采用与 macOS 版本一致的视觉样式：紧凑列表、圆角选中行、内嵌细分隔线、搜索匹配加粗，以及底部快捷键提示。快捷键：`Return` 复制所选条目，`↑`/`↓` 选择，`Esc` 清除搜索（搜索为空时再按一次隐藏窗口），`Ctrl+F` 聚焦搜索框，`Ctrl+1`/`Ctrl+2` 切换 History/Devices，`Ctrl+Q` 退出。

从仓库构建时，先安装 Wayland/xkbcommon/fontconfig 开发包和 `wl-clipboard`（提供 `wl-copy`、`wl-paste`）：

```sh
cargo build --release -p manis-pocket-linux -p manis-pocket-wayland
./target/release/manis-pocket
```

`manis-pocket-wayland` 命令行客户端保留供调试和无图形界面时使用，不要与图形应用同时运行。

Mac 端在设置中启用 **Clipboard Sync**。两台设备处于同一局域网时会通过 mDNS 发现；如发现失败，可在 Linux 端使用 `--connect MAC_IP:31774`，或在 Mac 的同步设置中手动连接 `LINUX_IP:31774`。确保 TCP 31774 可达。

在 Mac 的 **Discovered Devices** 或 Linux 的 **Devices** 页面点击 **Pair**。两端都会显示六位码；核对一致后在两端点击 **Confirm**。双方确认完成后，在任一设备复制纯文本，另一端的系统剪贴板会更新，可直接粘贴。Linux 图形应用的历史记录保存在本机；命令行客户端仍支持 `peers`、`pair`、`confirm`、`reject`、`unpair` 和 `quit`。

Linux 身份、已配对设备和当前剪贴板版本保存在 `~/.config/manis-pocket/`（或 `$XDG_CONFIG_HOME/manis-pocket/`）；图形应用的文本历史保存在 `~/.local/share/manis-pocket/history.json`（或 `$XDG_DATA_HOME/manis-pocket/history.json`）。新版配对协议要求重新配对旧设备。当前直接粘贴支持不超过 512 KiB 的 UTF-8 文本，以及不超过 10 MiB 的 PNG 图片（两端都会把图片规范化为 PNG，单条消息上限 20 MiB）；重连时会比较版本并补齐当前值。清空剪贴板会清空对端剪贴板；复制协议无法共享的内容（文件、超过上限的图片）不会改动对端已有内容。历史记录仍只在本机管理，文件和完整历史补齐尚未覆盖。文件下载入口暂时拒绝请求，避免远端按任意路径读取本机文件。

## GPUI 版（重写中）的构建与打包

新版本是一个 Rust/GPUI 应用，macOS 与 Linux 共用同一套代码：

```bash
cargo build -p manis-pocket-app          # 编译
./target/debug/manis-pocket --toggle     # 切换面板（可绑到合成器快捷键）
./target/debug/manis-pocket --settings   # 直接打开设置
./target/debug/manis-pocket --probe-app  # 诊断：当前前台应用/窗口类
```

**macOS 打包**（生成 `build/Manis Pocket.app`，`LSUIElement` 菜单栏应用）：

```bash
scripts/bundle-macos.sh                  # PROFILE=release 可出 release 包
```

**Linux 安装**（用户级，无需 root）：

```bash
scripts/install-linux.sh                 # 装二进制 + .desktop + 自启动
scripts/install-linux.sh --uninstall     # 全部撤销
```

调试说明：

- `RUST_LOG=debug` 打开核心与同步引擎的日志（含网络错误）。
- `MANIS_POCKET_DB=<路径>` 指定另一份数据库，便于并行测试互不干扰。
- `MANIS_POCKET_SYNC_PORT=<端口>` 换同步端口，便于和已安装的版本同时运行。
- 应用归属依赖系统接口：macOS 用 `NSWorkspace`（需以 .app 运行），Linux 用 `hyprctl`（Hyprland）。

### 面板位置（`popupPosition`）

| 值 | 行为 |
|---|---|
| `cursor`（默认） | 面板左上角跟随鼠标 |
| `center` | 屏幕居中 |
| `statusItem` / `window` / `lastPosition` | 暂未接线，退回右上角（菜单栏下方） |

**Linux/Wayland 重要限制**：xdg-shell 不允许客户端指定窗口绝对坐标，窗口位置由合成器决定。
所以在 Wayland 上 `cursor`/`center` **不会生效**，面板出现在合成器给的位置（Hyprland 下是浮动窗口的默认位置）。

要固定位置，请在 Hyprland 配置里按 window class 加规则：

```ini
windowrule = float, class:^(io.github.kaigedong.ManisPocket)$
windowrule = center, class:^(io.github.kaigedong.ManisPocket)$
```

macOS 上 `cursor` / `center` 是真正生效的（NSWindow 可以定位）。

## 快捷键与操作

| 操作 | 面板内 | 说明 |
|---|---|---|
| 搜索 | 直接输入 | 支持精确/模糊/正则/混合，设置里可切 |
| 选择 | ↑ / ↓ | |
| 复制并关闭 | 回车 | 复制当前条目 |
| 多选 | **⌘ + 点击** | 加入贴栈，行尾显示序号 |
| 复制整个贴栈 | 回车（多选后） | 按加入顺序用换行连接后复制 |
| 逐条粘贴 | 回车（多选 + 已开「选中即粘贴」） | 逐条写入剪贴板并发送粘贴键 |
| 固定/取消固定 | ⌥P | 行首显示固定字母 |
| 删除 | ⌥⌫ | |
| 预览 | ⌃Space | 图片条目显示缩略图 |
| 打开设置 | ⌘, | |
| 关闭 | esc | |

### 关于局域网可见性

同步开启后，设备通过 mDNS 在局域网内始终可见（用于自动发现）。引擎没有提供
"关闭广播但仍可显式连接"的能力，所以设置页没有这个开关——不留一个改了没效果的控件。

### 自动粘贴需要系统权限

「选中即粘贴」和「逐条粘贴」通过模拟系统粘贴快捷键实现，因此需要授权：

- **macOS**：系统设置 → 隐私与安全性 → 辅助功能，勾选 Manis Pocket。
  设置页在开关打开但未授权时会显示「自动粘贴需要辅助功能权限」和「去授权」按钮。
- **Linux (Wayland)**：需要安装 `wtype`（`pacman -S wtype`）。未安装时同样不生效。

未授权时复制功能正常，只是不会自动粘贴。

## 支持的内容类型

| 类型 | 记录 | 显示 | 复制回去 |
|---|---|---|---|
| 文本 | ✅ | 中间截断，显示应用图标 | ✅ |
| 图片（PNG） | ✅ 上限 10 MB | 行显示「图片」，预览显示缩略图 | ✅ |
| 文件 | ✅ | 显示文件名（多个显示「X 等 N 个文件」） | ✅ 以文件 URL 写回剪贴板 |
| 其他 | ❌ | — | — |
