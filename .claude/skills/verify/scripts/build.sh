#!/usr/bin/env bash
# Usage: build.sh
# Builds folio-app once and prints the binary path on stdout.
# Uses the shared-slot wrapper ~/.local/bin/cargo-slot when present.
set -euo pipefail

repo=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
cd "$repo"

export CARGO_INCREMENTAL=0
cargo_cmd=(cargo)
[ -x "$HOME/.local/bin/cargo-slot" ] && cargo_cmd=("$HOME/.local/bin/cargo-slot")

"${cargo_cmd[@]}" build -p folio-app >&2
echo "${CARGO_TARGET_DIR:-$repo/target}/debug/folio-app"
