#!/usr/bin/env bash
set -euo pipefail

# Build Rust core for macOS, generate Swift bindings, copy to project.
echo "==> Building Rust core"
cargo build --release
ls -lh target/release/libmanis_pocket_core.a target/release/libmanis_pocket_sync.a

echo "==> Generating UniFFI Swift bindings"
cargo run --release --bin uniffi-bindgen --package manis-pocket-core generate \
  --library target/release/libmanis_pocket_core.dylib \
  --language swift \
  --out-dir ManisPocket/Generated

cp ManisPocket/Generated/ManisPocketCore.swift ManisPocket/ManisPocketCore.swift
echo "==> Swift bindings generated"
