#!/usr/bin/env bash
# Usage: smoke.sh <env-file>
# API-level smoke test against a running ephemeral instance (see up.sh):
# upload -> processed -> list/get -> download -> SSE event -> delete.
# Exits non-zero on the first failed check.
set -euo pipefail

env_file=${1:?usage: smoke.sh <env-file>}
# shellcheck disable=SC1090
source "$env_file"

here=$(cd "$(dirname "$0")" && pwd)
repo=$(git -C "$here" rev-parse --show-toplevel)
fixture=$repo/e2e/fixtures/sample.pdf
title="smoke-$$-$(date +%s)"
sse_out=$VERIFY_DIR/sse.txt
download=$VERIFY_DIR/download.pdf

step() { echo "==> $*"; }
fail() { echo "FAIL: $*" >&2; exit 1; }
jqpy() { python3 -c "import json,sys; d=json.load(sys.stdin); print($1)"; }

step "open SSE stream"
curl -sN --max-time 60 "$BASE_URL/events" >"$sse_out" &
sse_pid=$!
trap 'kill "$sse_pid" 2>/dev/null || true' EXIT
sleep 0.5

step "upload $title"
curl -fsS -o /dev/null -F "title=$title" -F "file=@$fixture;type=application/pdf" \
  "$BASE_URL/documents/upload"

step "find it via GET /api/documents"
id=$(curl -fsS "$BASE_URL/api/documents" |
  jqpy "next(x['id'] for x in d['documents'] if x['title']=='$title')") ||
  fail "uploaded document not listed"
echo "id=$id"

step "wait for processing (content set by the worker)"
# Extension point: once a status/processing field exists, poll that instead.
for _ in $(seq 1 60); do
  content=$(curl -fsS "$BASE_URL/api/documents/$id" | jqpy "d.get('content') is not None")
  [ "$content" = True ] && break
  sleep 0.5
done
[ "$content" = True ] || fail "document not processed within 30s"

step "GET /api/documents/$id"
curl -fsS "$BASE_URL/api/documents/$id" |
  jqpy "'ok' if d['id']=='$id' and d['title']=='$title' and d['mime_type']=='application/pdf' else sys.exit('unexpected shape: %r' % d)"

step "download"
curl -fsS -o "$download" "$BASE_URL/api/documents/$id/download"
cmp -s "$download" "$fixture" || fail "downloaded bytes differ from fixture"

step "assert an SSE event arrived"
grep -q '^event:' "$sse_out" || fail "no SSE event received"
grep '^event:' "$sse_out" | sort -u | sed 's/^/  /'

step "delete"
curl -fsS -o /dev/null -X DELETE "$BASE_URL/api/documents/$id"
code=$(curl -s -o /dev/null -w '%{http_code}' "$BASE_URL/api/documents/$id")
[ "$code" = 404 ] || fail "expected 404 after delete, got $code"

echo "smoke: PASS"
