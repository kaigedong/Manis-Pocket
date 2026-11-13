# 开发

macOS / Linux 桌面端都是同一个 Rust 程序（GPUI），只需要 Rust：

```sh
cargo run -p manis-pocket-app                 # 直接运行
bash scripts/bundle-macos.sh                  # 打成 .app（默认 debug）
PROFILE=release bash scripts/bundle-macos.sh   # Release 包
scripts/linux-dev.sh build                    # 在 Linux 主机上构建（需要 Wayland 依赖）
```

Android 构建需要 Android SDK/NDK 和 Rust Android target：

```sh
bash scripts/build-rust-android.sh arm64
cd android && ./gradlew assembleDebug
```
