#!/usr/bin/env bash
# Everything CI verifies, runnable on one machine.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "=== Format check ==="
cargo fmt --check && echo "✅ Rust fmt"
./ktlint 'android/**/*.kt' '!android/**/manis_pocket_core.kt' 2>&1 | tail -1
python3 -m unittest discover -s scripts/tests 2>&1 | tail -3
echo ""

echo "=== Rust ==="
cargo check
cargo test -p manis-pocket-core
cargo test -p manis-pocket-sync
cargo test -p manis-pocket-app
echo ""

echo "=== macOS bundle ==="
PROFILE=release bash scripts/bundle-macos.sh
echo ""

echo "=== Android ==="
cd android && export JAVA_HOME=/opt/homebrew/opt/openjdk@21 && export ANDROID_HOME=$HOME/Android/sdk && export ANDROID_NDK_HOME=$ANDROID_HOME/ndk/27.0.12077973
cargo ndk -t arm64-v8a -o app/src/main/jniLibs build --release --package manis-pocket-core
cargo run --release --bin uniffi-bindgen --package manis-pocket-core generate --library ~/.cargo/target/aarch64-linux-android/release/libmanis_pocket_core.so --language kotlin --out-dir app/src/main/java
./gradlew assembleDebug
cd ..

echo ""
echo "✅ All checks passed"
