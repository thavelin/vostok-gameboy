#!/usr/bin/env bash
set -euo pipefail
cd -- "$(dirname -- "$(readlink -f -- "$0")")"
# With Cargo, keep the executable current (a no-op when nothing changed).
# Without Cargo, run an existing build as-is.
start="$OLDPWD"
if command -v cargo >/dev/null 2>&1; then
  cargo build --release --locked --quiet
elif [[ ! -x target/release/vostok-gameboy ]]; then
  echo "ERROR: Not built and cargo is not installed. See README.md." >&2
  exit 1
fi
app="$PWD/target/release/vostok-gameboy"
cd -- "$start"
exec "$app" "$@"
