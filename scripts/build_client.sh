#!/usr/bin/env sh
set -eu

ROOT="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cd "$ROOT"

cargo build --release -p queensgame-client --target wasm32-unknown-unknown
rm -rf dist/client
mkdir -p dist/client
wasm-bindgen \
  --target web \
  --out-dir dist/client \
  --out-name queensgame_client \
  target/wasm32-unknown-unknown/release/queensgame_client.wasm
