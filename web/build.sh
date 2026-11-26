#!/usr/bin/env bash
# Build the typesetter for the browser into web/www/pkg.
#
# Needs the wasm32-unknown-unknown target and wasm-bindgen's CLI at the
# version web/Cargo.toml pins:
#   rustup target add wasm32-unknown-unknown
#   cargo install wasm-bindgen-cli --version 0.2.130
# Then serve web/www over HTTP (python3 -m http.server -d web/www).
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
cargo build --release -p calamus-web --target wasm32-unknown-unknown
"${WASM_BINDGEN:-wasm-bindgen}" --target web --no-typescript --out-dir web/www/pkg \
  target/wasm32-unknown-unknown/release/calamus_web.wasm
cp fonts/Inter-Regular.ttf web/www/pkg/
