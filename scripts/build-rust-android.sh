#!/usr/bin/env bash
set -euo pipefail

# Build Rust core for Android, generate Kotlin bindings, fix generated code.
# Usage: bash scripts/build-rust-android.sh [arm64|all]
#   arm64 — aarch64 only (fast, for PRs)
#   all   — all 4 ABIs (for releases)

MODE="${1:-arm64}"
ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-$ANDROID_HOME/ndk/27.0.12077973}"
export ANDROID_NDK_HOME

echo "ANDROID_HOME=$ANDROID_HOME"
echo "ANDROID_NDK_HOME=$ANDROID_NDK_HOME"
ls "$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/linux-x86_64/bin/" | head -5

if [[ "$MODE" == "all" ]]; then
  TARGETS=(arm64-v8a armeabi-v7a x86_64 x86)
else
  TARGETS=(arm64-v8a)
fi

for TARGET in "${TARGETS[@]}"; do
  echo "==> Building Rust for $TARGET"
  cargo ndk \
    -t "$TARGET" \
    -o android/app/src/main/jniLibs \
    build --release \
    --package manis-pocket-core
done

echo "==> Generating UniFFI Kotlin bindings"
LIB="android/app/src/main/jniLibs/arm64-v8a/libmanis_pocket_core.so"
ls -lh "$LIB"

cargo run --release \
  --bin uniffi-bindgen \
  --package manis-pocket-core \
  generate \
  --library "$LIB" \
  --language kotlin \
  --out-dir android/app/src/main/java

echo "Generated Kotlin files:"
find android/app/src/main/java -name "*.kt" -o -name "*.java" | sort

echo "==> Fixing UniFFI generated code"
# CoreError subclasses have 'val `message`' that shadows Throwable.message.
# Remove 'val' from constructor param so 'override val message' getter works alone.
sed -i 's/val `message`: kotlin\.String/`message`: kotlin.String/g' \
  android/app/src/main/java/com/kaigedong/manispocket/manis_pocket_core.kt
echo "✅ Android Rust build complete"
