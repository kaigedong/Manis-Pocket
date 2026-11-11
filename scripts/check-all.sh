#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."

echo "=== Format check ==="
cargo fmt --check && echo "✅ Rust fmt"
swiftformat --lint ManisPocket/ ManisPocketTests/ ManisPocketUITests/ 2>&1 | tail -1
./ktlint 'android/**/*.kt' '!android/**/manis_pocket_core.kt' 2>&1 | tail -1
echo ""

echo "=== Rust ==="
cargo check
cargo test -p manis-pocket-core
cargo build --release -p manis-pocket-core
echo ""

echo "=== macOS ==="
cp ~/.cargo/target/release/libmanis_pocket_core.a ~/.cargo/target/release/libmanis_pocket_core.dylib ~/.cargo/target/release/libmanis_pocket_sync.a target/release/ 2>/dev/null || true
cargo run --release --bin uniffi-bindgen --package manis-pocket-core generate --library ~/.cargo/target/release/libmanis_pocket_core.dylib --language swift --out-dir ManisPocket/Generated
cp ManisPocket/Generated/ManisPocketCore.swift ManisPocket/ManisPocketCore.swift
xcodebuild build -project ManisPocket.xcodeproj -scheme ManisPocket -configuration Release -derivedDataPath build ARCHS=arm64 ONLY_ACTIVE_ARCH=NO CODE_SIGN_IDENTITY="-" CODE_SIGNING_REQUIRED=NO CODE_SIGNING_ALLOWED=NO
echo ""

echo "=== Android ==="
cd android && export JAVA_HOME=/opt/homebrew/opt/openjdk@21 && export ANDROID_HOME=$HOME/Android/sdk && export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/27.0.12077973
cargo ndk -t arm64-v8a -o app/src/main/jniLibs build --release --package manis-pocket-core
cargo run --release --bin uniffi-bindgen --package manis-pocket-core generate --library ~/.cargo/target/aarch64-linux-android/release/libmanis_pocket_core.so --language kotlin --out-dir app/src/main/java
./gradlew assembleDebug
cd ..

echo ""
echo "✅ All checks passed"
