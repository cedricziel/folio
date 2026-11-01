#!/usr/bin/env bash
# Usage: down.sh <env-file>
# Stops the instance named in the env file and removes its temp dir. Idempotent.
set -euo pipefail

env_file=${1:?usage: down.sh <env-file>}
[ -f "$env_file" ] || exit 0
# shellcheck disable=SC1090
source "$env_file"

if kill -0 "$PID" 2>/dev/null; then
  kill "$PID" 2>/dev/null || true
  for _ in $(seq 1 20); do
    kill -0 "$PID" 2>/dev/null || break
    sleep 0.25
  done
  kill -9 "$PID" 2>/dev/null || true
fi

case "$(basename "$VERIFY_DIR")" in
  folio-verify.*) rm -rf "$VERIFY_DIR" ;;
  *) echo "down.sh: refusing to remove unexpected dir: $VERIFY_DIR" >&2; exit 1 ;;
esac
echo "down.sh: stopped $PID, removed $VERIFY_DIR" >&2
