#!/usr/bin/env bash
# One-time (or repeat) setup for the stick9p ESP32-S3 Rust workspace.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

echo "==> Checking rustup..."
command -v rustup >/dev/null || { echo "Install rustup from https://rustup.rs/"; exit 1; }

echo "==> Installing cargo tools..."
cargo install espup espflash flip-link ldproxy esp-generate --locked 2>/dev/null || true

echo "==> Installing ESP Xtensa toolchain (esp32s3)..."
if ! rustup toolchain list | grep -q '^esp$'; then
  espup install --toolchain-version 1.95.0.0 --skip-version-parse --targets esp32s3
else
  echo "    esp toolchain already present"
fi

EXPORT="${HOME}/export-esp.sh"
if [[ -f "$EXPORT" ]]; then
  # shellcheck source=/dev/null
  source "$EXPORT"
  echo "==> Sourced $EXPORT"
else
  echo "WARN: $EXPORT not found — run: source ~/export-esp.sh"
fi

echo "==> Building firmware..."
cargo build -p firmware

echo "==> Building host bridge..."
# Build outside the ESP workspace tree so the host toolchain is used (not xtensa).
( cd "$HOME" && cargo build --manifest-path "$ROOT/tools/stick9p-bridge/Cargo.toml" )

echo ""
echo "Setup complete. In new shells run:  source ~/export-esp.sh"
echo "Flash firmware:                   cargo run -p firmware"
