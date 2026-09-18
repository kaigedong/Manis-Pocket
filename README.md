# Manis Pocket

复制过的，随手找回。

Manis Pocket 是轻量的剪贴板历史工具，支持快速搜索、固定常用内容，以及在 macOS 与 Android 设备之间通过本地网络加密同步。

## 安装

从本仓库的 [Releases](../../releases) 下载。macOS 需要 14 或更新版本；Android 版本仍在开发中。

## 使用

- 在 macOS 上按 `⇧⌘C` 打开历史记录，输入关键词搜索。
- 按 `Return` 复制所选内容；按 `⌥Return` 复制并粘贴。
- 按 `⌥P` 固定或取消固定所选项目。
- 在设置中管理历史记录、快捷键和设备同步。

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
