#!/usr/bin/env bash
# Usage: up.sh
# Starts an ephemeral Folio instance (own port, DB, storage) and prints the
# path of an env file defining BASE_URL, PID and VERIFY_DIR on stdout.
# Binary: $FOLIO_BIN, else the debug build (run build.sh first; up.sh builds it if missing).
# Tear down with down.sh <env-file>.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=${FOLIO_REPO:-$(git -C "$here" rev-parse --show-toplevel)}
bin=${FOLIO_BIN:-${CARGO_TARGET_DIR:-$repo/target}/debug/folio-app}
[ -x "$bin" ] || bin=$("$here/build.sh")
[ -x "$bin" ] || { echo "up.sh: binary not found: $bin" >&2; exit 1; }

dir=$(mktemp -d "${TMPDIR:-/tmp}/folio-verify.XXXXXX")
port=$(python3 -c 'import socket; s=socket.socket(); s.bind(("127.0.0.1",0)); print(s.getsockname()[1])')
base="http://127.0.0.1:$port"
mkdir -p "$dir/storage"

# The app serves ./static relative to its working directory.
ln -s "$repo/static" "$dir/static"

# Run from $dir with a scrubbed environment: Config::load() reads ./config.toml
# and dotenvy walks up from the cwd looking for .env, so neither the repo's nor
# the user's files are ever seen. FOLIO_CONFIG is left unset for the same reason.
(
  cd "$dir"
  exec env -i PATH="$PATH" HOME="$dir" RUST_LOG="${RUST_LOG:-info}" \
    FOLIO_SERVER__HOST=127.0.0.1 \
    FOLIO_SERVER__PORT="$port" \
    FOLIO_DATABASE__URL="sqlite:$dir/folio.db?mode=rwc" \
    FOLIO_STORAGE__PATH="$dir/storage" \
    FOLIO_AI__PROVIDER=local \
    FOLIO_AUTH__REQUIRED="${FOLIO_VERIFY_AUTH_REQUIRED:-false}" \
    "$bin"
) >"$dir/server.log" 2>&1 &
pid=$!

printf 'BASE_URL=%s\nPID=%s\nVERIFY_DIR=%s\n' "$base" "$pid" "$dir" >"$dir/env"

# Readiness: /readyz when the health-endpoints PR has landed (404 before), else /.
for _ in $(seq 1 60); do
  if ! kill -0 "$pid" 2>/dev/null; then
    echo "up.sh: server exited early; log:" >&2
    tail -n 30 "$dir/server.log" >&2
    "$here/down.sh" "$dir/env" >&2
    exit 1
  fi
  code=$(curl -s -o /dev/null -w '%{http_code}' "$base/readyz" || true)
  [ "$code" = 404 ] && code=$(curl -s -o /dev/null -w '%{http_code}' "$base/" || true)
  if [ "$code" = 200 ]; then
    echo "up.sh: ready at $base (pid $pid)" >&2
    echo "$dir/env"
    exit 0
  fi
  sleep 0.5
done

echo "up.sh: not ready after 30s; log:" >&2
tail -n 30 "$dir/server.log" >&2
"$here/down.sh" "$dir/env" >&2
exit 1
