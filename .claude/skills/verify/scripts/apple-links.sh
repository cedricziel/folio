#!/usr/bin/env bash
# Usage: apple-links.sh <env-file>
# Checks /.well-known/apple-app-site-association and the /settings section.
# With FOLIO_VERIFY_APPLE_APP_IDS set when the instance started, expects 200 JSON
# with the masked components; without it, expects 404. Auth must be off.
set -euo pipefail

# shellcheck disable=SC1090
source "$1"
out="$VERIFY_DIR/apple-links"
mkdir -p "$out"
fail() { echo "apple-links: FAIL: $*" >&2; exit 1; }

url="$BASE_URL/.well-known/apple-app-site-association"
code=$(curl -sS -D "$out/headers" -o "$out/body" -w '%{http_code}' "$url")
echo "GET /.well-known/apple-app-site-association -> $code"

if [ -z "${FOLIO_VERIFY_APPLE_APP_IDS:-}" ]; then
  [ "$code" = 404 ] || fail "expected 404 without app IDs, got $code"
else
  [ "$code" = 200 ] || fail "expected 200, got $code"
  grep -qi '^content-type: application/json' "$out/headers" || fail "content type"
  grep -qi '^cache-control: public, max-age=3600' "$out/headers" || fail "cache control"
  ! grep -qi '^set-cookie' "$out/headers" || fail "unexpected Set-Cookie"
  python3 - "$out/body" "$FOLIO_VERIFY_APPLE_APP_IDS" <<'PY'
import json, sys

def check(ok, message):
    if not ok:
        print(f"apple-links: FAIL: {message}", file=sys.stderr)
        sys.exit(1)

body = json.load(open(sys.argv[1]))
u = "????????-????-????-????-????????????"
expected_ids = [i.strip() for i in sys.argv[2].split(",") if i.strip()]
detail = body["applinks"]["details"][0]
check(detail["appIDs"] == expected_ids, f"appIDs {detail['appIDs']}")
patterns = [c["/"] for c in detail["components"]]
check(patterns == [f"/vaults/{u}/documents/{u}", f"/vaults/{u}/saved-views/{u}", f"/vaults/{u}/"], f"components {patterns}")
raw = json.dumps(body)
check("*" not in raw and "exclude" not in raw, f"wildcard or exclude in {raw}")
PY
fi

legacy=$(curl -sS -o /dev/null -w '%{http_code}' "$BASE_URL/apple-app-site-association")
[ "$legacy" != 200 ] || fail "legacy root path is served"

curl -sS "$BASE_URL/settings" >"$out/settings.html"
grep -q 'Apple app links' "$out/settings.html" || fail "/settings has no Apple app links section"
grep -q "CDN refreshes" "$out/settings.html" || fail "/settings lacks the CDN note"
grep -q "$url" "$out/settings.html" || fail "/settings lacks the association URL"
echo "apple-links: OK"
