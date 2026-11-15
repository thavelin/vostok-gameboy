#!/usr/bin/env bash
# Builds the release binary into target/. No system packages are touched
# (on Linux the sound library needs the ALSA development package).
set -euo pipefail
cd -- "$(dirname -- "$(readlink -f -- "$0")")"
if ! command -v cargo >/dev/null 2>&1; then
  echo "Rust is required: https://rustup.rs" >&2
  exit 1
fi
cargo build --release --locked
echo "ready: ./run.sh"
