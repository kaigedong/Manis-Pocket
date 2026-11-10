# 开发

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
