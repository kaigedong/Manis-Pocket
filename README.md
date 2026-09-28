# Manis Pocket

复制过的，随手找回。

Manis Pocket 是轻量的剪贴板历史工具，支持快速搜索、固定常用内容，以及在 macOS 与 Linux Wayland 设备之间通过本地网络同步文本剪贴板。Android 版本仍在开发中。

## 安装

从本仓库的 [Releases](../../releases) 下载。macOS 需要 14 或更新版本；Android 版本仍在开发中。

## 使用

- 在 macOS 上按 `⇧⌘C` 打开历史记录，输入关键词搜索。
- 按 `Return` 复制所选内容；按 `⌥Return` 复制并粘贴。
- 按 `⌥P` 固定或取消固定所选项目。
- 在设置中管理历史记录、快捷键和设备同步。

### macOS ↔ Linux Wayland 直接粘贴

Linux 端使用 Rust 命令行客户端，不依赖 Tauri。先在 Wayland 会话中安装 `wl-clipboard`（提供 `wl-copy`、`wl-paste`），再从仓库构建：

```sh
cargo build --release -p manis-pocket-wayland
./target/release/manis-pocket-wayland --name "Linux Laptop"
```

Arch Linux 可用 [本地 PKGBUILD](packaging/archlinux/README.md) 构建安装；`manis-pocket-bin` [AUR 二进制包](packaging/aur/README.md) 将安装同一个 Wayland 命令行客户端。

Mac 端在设置中启用 **Clipboard Sync**。两台设备处于同一局域网时会通过 mDNS 发现；如发现失败，可在 Linux 端使用 `--connect MAC_IP:31774`，或在 Mac 的同步设置中手动连接 `LINUX_IP:31774`。确保 TCP 31774 可达。

在 Mac 的 **Discovered Devices** 点击 **Pair**，或在 Linux 客户端输入 `peers` 查看设备 ID，再输入 `pair PEER_ID`。两端都会显示六位码；核对一致后在 Mac 点击 **Confirm**，在 Linux 输入 `confirm PEER_ID 六位码`。双方确认完成后，在任一设备复制纯文本，另一端的系统剪贴板会更新，可直接粘贴。Linux 客户端还支持 `reject PEER_ID`、`unpair PEER_ID` 和 `quit`。

Linux 身份、已配对设备和当前剪贴板版本保存在 `~/.config/manis-pocket/`（或 `$XDG_CONFIG_HOME/manis-pocket/`）。新版配对协议要求重新配对旧设备。当前直接粘贴支持不超过 512 KiB 的 UTF-8 文本；重连时会比较版本并补齐当前值。清空剪贴板或复制不可共享内容会废止旧文本并清空对端剪贴板。历史记录仍只在本机管理，图片、文件和完整历史补齐尚未覆盖。文件下载入口暂时拒绝请求，避免远端按任意路径读取本机文件。

## 开发

macOS 构建需要 Xcode 和 Rust：

```sh
bash scripts/build-rust-macos.sh
xcodebuild build -project ManisPocket.xcodeproj -scheme ManisPocket -configuration Debug
```

Android 构建需要 Android SDK/NDK 和 Rust Android target：

```sh
bash scripts/build-rust-android.sh arm64
cd android && ./gradlew assembleDebug
```

## 许可证

MIT，详见 [LICENSE](LICENSE)。保留原作者 Alex Rodionov 的版权声明。
