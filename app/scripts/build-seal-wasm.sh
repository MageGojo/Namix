#!/usr/bin/env bash
# 编译 namix-seal → views/generated/seal（供 callRust 动态 import）
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
OUT="$ROOT/app/src/views/generated/seal"
cd "$ROOT"

rustup target add wasm32-unknown-unknown >/dev/null 2>&1 || true

# 与 crates/namix-seal 的 wasm-bindgen 版本对齐
WB_VER="$(cargo tree -p namix-seal -i wasm-bindgen --depth 0 2>/dev/null | sed -n 's/.*wasm-bindgen v\([0-9.]*\).*/\1/p' | head -1)"
WB_VER="${WB_VER:-0.2.126}"
if ! wasm-bindgen -V 2>/dev/null | grep -q "$WB_VER"; then
  echo "installing wasm-bindgen-cli $WB_VER…"
  cargo install wasm-bindgen-cli --version "$WB_VER" --force --locked
fi

cargo build -p namix-seal --target wasm32-unknown-unknown --release
mkdir -p "$OUT"
wasm-bindgen \
  "$ROOT/target/wasm32-unknown-unknown/release/namix_seal.wasm" \
  --out-dir "$OUT" \
  --target web

# 给 Vite 一个稳定入口名
if [[ -f "$OUT/namix_seal.js" ]]; then
  echo "✓ wasm → $OUT"
else
  echo "wasm-bindgen output missing" >&2
  exit 1
fi
