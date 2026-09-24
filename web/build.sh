#!/usr/bin/env bash
# Build the engine for the browser: web/pkg/eelisp_web.js + eelisp_web_bg.wasm.
#
#   web/build.sh            release build (small, slow to compile)
#   web/build.sh --dev      debug build
#
# Needs: rustup target add wasm32-unknown-unknown
#        cargo install wasm-bindgen-cli --version 0.2.126   (must match wasm-bindgen in Cargo.toml)
#
# SQLite is C, compiled to WebAssembly by sqlite-wasm-rs with the system clang. On macOS the
# system `ar` quietly drops WebAssembly objects from the archive — the link then fails with
# "undefined symbol: sqlite3_step" — so the archiver is the llvm-ar that ships with Rust
# (rustup component add llvm-tools).
set -euo pipefail
cd "$(dirname "$0")"

AR="$(ls "$(rustc --print sysroot)"/lib/rustlib/*/bin/llvm-ar 2>/dev/null | head -1 || true)"
if [ -z "$AR" ]; then
  echo "llvm-ar not found: rustup component add llvm-tools" >&2
  exit 1
fi
export AR_wasm32_unknown_unknown="$AR"
# SQLite's WebAssembly shim writes C23 attributes ([[noreturn]]). A recent clang takes them in any mode;
# clang 14 (Ubuntu 22.04) only in C2x mode. gnu2x works on both.
export CFLAGS_wasm32_unknown_unknown="${CFLAGS_wasm32_unknown_unknown:--std=gnu2x}"

PROFILE=release
FLAG=--release
if [ "${1:-}" = "--dev" ]; then PROFILE=debug; FLAG=; fi

cargo build $FLAG --target wasm32-unknown-unknown
wasm-bindgen --target web --out-dir pkg "target/wasm32-unknown-unknown/$PROFILE/eelisp_web.wasm"
ls -la pkg/*.wasm
