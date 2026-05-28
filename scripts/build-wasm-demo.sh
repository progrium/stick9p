#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
wat2wasm="${WAT2WASM:-wat2wasm}"
"$wat2wasm" "$root/wasm/hello.wat" -o "$root/wasm/demo.wasm"
echo "wrote $root/wasm/demo.wasm ($(wc -c <"$root/wasm/demo.wasm") bytes)"
