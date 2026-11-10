# 使用

- 在 macOS 上按 `⇧⌘C` 打开历史记录，输入关键词搜索。
- 按 `Return` 复制所选内容；按 `⌥Return` 复制并粘贴。
- 按 `⌥P` 固定或取消固定所选项目。
- 在设置中管理历史记录、快捷键和设备同步。

## macOS ↔ Linux Wayland 直接粘贴

Linux 端使用原生 GTK4 图形应用和 Rust 同步引擎，不依赖 Tauri。Arch Linux 安装 `manis-pocket-bin` 后，从应用菜单打开 **Manis Pocket**，也可以在 Wayland 会话中运行 `manis-pocket`。窗口的 **History** 可搜索和重新复制文本，**Devices** 可发现、连接和配对设备。关闭窗口后同步仍在后台运行；重新从应用菜单打开窗口，点击 **Quit** 才会退出。

Linux 窗口采用与 macOS 版本一致的视觉样式：紧凑列表、圆角选中行、内嵌细分隔线、搜索匹配加粗，以及底部快捷键提示。快捷键：`Return` 复制所选条目，`↑`/`↓` 选择，`Esc` 清除搜索（搜索为空时再按一次隐藏窗口），`Ctrl+F` 聚焦搜索框，`Ctrl+1`/`Ctrl+2` 切换 History/Devices，`Ctrl+Q` 退出。

从仓库构建时，先安装 GTK4 开发包和 `wl-clipboard`（提供 `wl-copy`、`wl-paste`）：

```sh
cargo build --release -p manis-pocket-linux -p manis-pocket-wayland
./target/release/manis-pocket
```

`manis-pocket-wayland` 命令行客户端保留供调试和无图形界面时使用，不要与图形应用同时运行。

Mac 端在设置中启用 **Clipboard Sync**。两台设备处于同一局域网时会通过 mDNS 发现；如发现失败，可在 Linux 端使用 `--connect MAC_IP:31774`，或在 Mac 的同步设置中手动连接 `LINUX_IP:31774`。确保 TCP 31774 可达。

在 Mac 的 **Discovered Devices** 或 Linux 的 **Devices** 页面点击 **Pair**。两端都会显示六位码；核对一致后在两端点击 **Confirm**。双方确认完成后，在任一设备复制纯文本，另一端的系统剪贴板会更新，可直接粘贴。Linux 图形应用的历史记录保存在本机；命令行客户端仍支持 `peers`、`pair`、`confirm`、`reject`、`unpair` 和 `quit`。

Linux 身份、已配对设备和当前剪贴板版本保存在 `~/.config/manis-pocket/`（或 `$XDG_CONFIG_HOME/manis-pocket/`）；图形应用的文本历史保存在 `~/.local/share/manis-pocket/history.json`（或 `$XDG_DATA_HOME/manis-pocket/history.json`）。新版配对协议要求重新配对旧设备。当前直接粘贴支持不超过 512 KiB 的 UTF-8 文本，以及不超过 10 MiB 的 PNG 图片（两端都会把图片规范化为 PNG，单条消息上限 20 MiB）；重连时会比较版本并补齐当前值。清空剪贴板会清空对端剪贴板；复制协议无法共享的内容（文件、超过上限的图片）不会改动对端已有内容。历史记录仍只在本机管理，文件和完整历史补齐尚未覆盖。文件下载入口暂时拒绝请求，避免远端按任意路径读取本机文件。
