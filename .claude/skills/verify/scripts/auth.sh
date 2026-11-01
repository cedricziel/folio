#!/usr/bin/env bash
# Usage: auth.sh <env-file>
# Auth checks against an instance started with FOLIO_VERIFY_AUTH_REQUIRED=true.
set -euo pipefail

env_file=${1:?usage: auth.sh <env-file>}
# shellcheck disable=SC1090
source "$env_file"

step() { echo "==> $*"; }
fail() { echo "FAIL: $*" >&2; exit 1; }
code() { curl -s -o /dev/null -w '%{http_code}' "$@"; }
expect() { [ "$1" = "$2" ] || fail "$3: expected $2, got $1"; echo "ok  $3 ($1)"; }

jar=$VERIFY_DIR/cookies.txt
token=$(grep -o 'setup?token=[A-Za-z0-9_-]*' "$VERIFY_DIR/server.log" | head -1 | cut -d= -f2)
[ -n "$token" ] || fail "no setup link in server.log"
echo "setup link logged: $BASE_URL/setup?token=<redacted>"

step "anonymous access is refused"
expect "$(code "$BASE_URL/setup")" 403 "GET /setup without token"
expect "$(code "$BASE_URL/setup?token=wrong")" 403 "GET /setup with a wrong token"
expect "$(code "$BASE_URL/api/documents")" 401 "GET /api/documents"
expect "$(code "$BASE_URL/")" 303 "GET / redirects to login"
expect "$(code -X POST -F 'files=@/dev/null' "$BASE_URL/share")" 401 "anonymous POST /share"
expect "$(code "$BASE_URL/manifest.webmanifest")" 200 "manifest is public"
expect "$(code "$BASE_URL/sw.js")" 200 "service worker is public"

step "protected-resource metadata and challenge for /api"
curl -fsS "$BASE_URL/.well-known/oauth-protected-resource/api" | grep -q "\"resource\":\"$BASE_URL/api\"" ||
  fail "api resource metadata"
curl -sI "$BASE_URL/api/documents" | grep -qi "oauth-protected-resource/api" || fail "challenge does not name /api metadata"
echo "ok  /api metadata + challenge"

step "create the owner with the setup token"
page=$(curl -fsS -c "$jar" "$BASE_URL/setup?token=$token")
csrf=$(printf '%s' "$page" | grep -o 'name="csrf" value="[^"]*"' | head -1 | sed 's/.*value="//; s/"$//')
expect "$(code -b "$jar" -c "$jar" -d "csrf=$csrf" -d "token=wrong" -d username=owner -d password=verify-password-123 "$BASE_URL/setup")" 403 "POST /setup with a wrong token"
expect "$(code -b "$jar" -c "$jar" -d "csrf=$csrf" -d "token=$token" -d username=owner -d password=verify-password-123 "$BASE_URL/setup")" 303 "POST /setup with the token"
expect "$(code -b "$jar" "$BASE_URL/api/documents")" 200 "session reads /api"
expect "$(code "$BASE_URL/setup?token=$token")" 303 "setup is closed afterwards"

step "browser-style hardening"
curl -sI "$BASE_URL/login" | grep -qi '^x-frame-options: deny' || fail "login lacks X-Frame-Options"
curl -sI "$BASE_URL/login" | grep -qi "^content-security-policy: frame-ancestors 'none'" || fail "login lacks frame-ancestors"
echo "ok  login page headers"
expect "$(code -b "$jar" -X POST -H 'Sec-Fetch-Site: cross-site' -H 'Content-Type: application/json' -d '{"name":"x"}' "$BASE_URL/api/tags")" 403 "cross-site write with the session cookie"
expect "$(code -b "$jar" -X POST -H 'Sec-Fetch-Site: same-origin' -H 'Content-Type: application/json' -d '{"name":"verify-tag"}' "$BASE_URL/api/tags")" 200 "same-origin write"

step "failed logins are throttled"
page=$(curl -fsS -c "$VERIFY_DIR/login.jar" "$BASE_URL/login")
csrf=$(printf '%s' "$page" | grep -o 'name="csrf" value="[^"]*"' | head -1 | sed 's/.*value="//; s/"$//')
last=
for _ in 1 2 3 4 5 6; do
  last=$(code -b "$VERIFY_DIR/login.jar" -d "csrf=$csrf" -d username=owner -d password=wrong "$BASE_URL/login")
done
expect "$last" 429 "6th failed login"

step "registration is rate limited per peer"
status=
for i in $(seq 1 30); do
  status=$(code -X POST -H 'Content-Type: application/json' -d '{"redirect_uris":["https://a.example/cb"]}' "$BASE_URL/oauth/register")
  [ "$status" = 429 ] && break
done
expect "$status" 429 "registration burst ends in 429 (request $i)"

echo "auth checks passed"
