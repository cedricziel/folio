#!/usr/bin/env bash
# Writes Resources/Folio-Local.entitlements for a self-build with Universal Links: the entitlements
# XcodeGen generated, plus com.apple.developer.associated-domains with one applinks: entry per host.
#
# Hosts come from FOLIO_ASSOCIATED_DOMAINS in the environment or, when that is unset, from
# Config/Local.xcconfig, separated by spaces or commas (e.g. "folio.example.com" or
# "folio.lan?mode=developer"). With no hosts the script does nothing.
#
# Run after `xcodegen generate`; point FOLIO_APP_ENTITLEMENTS at Resources/Folio-Local.entitlements.
set -euo pipefail

folio_dir="$(cd "$(dirname "$0")/../Folio" && pwd)"
source_file="$folio_dir/Resources/Folio.entitlements"
target_file="$folio_dir/Resources/Folio-Local.entitlements"
plistbuddy="${PLISTBUDDY:-/usr/libexec/PlistBuddy}"
key=com.apple.developer.associated-domains

if [ -z "${FOLIO_ASSOCIATED_DOMAINS+set}" ] && [ -f "$folio_dir/Config/Local.xcconfig" ]; then
    # The last assignment wins, as in Xcode; a trailing // comment is not part of the value.
    FOLIO_ASSOCIATED_DOMAINS="$(sed -n 's/^[[:space:]]*FOLIO_ASSOCIATED_DOMAINS[[:space:]]*=\(.*\)$/\1/p' \
        "$folio_dir/Config/Local.xcconfig" | tail -n 1 | sed 's|//.*$||')"
fi
hosts=()
# read -a splits without globbing, so "folio.lan?mode=developer" stays as written.
read -r -a words <<<"${FOLIO_ASSOCIATED_DOMAINS//,/ }"
for host in ${words[@]+"${words[@]}"}; do
    hosts+=("${host#applinks:}")
done

if [ "${#hosts[@]}" -eq 0 ]; then
    echo "FOLIO_ASSOCIATED_DOMAINS is empty; no associated domains to add."
    exit 0
fi
if [ ! -f "$source_file" ]; then
    echo "error: $source_file is missing; run xcodegen generate first." >&2
    exit 1
fi

cp "$source_file" "$target_file"
"$plistbuddy" -c "Delete :$key" "$target_file" >/dev/null 2>&1 || true
"$plistbuddy" -c "Add :$key array" "$target_file"
index=0
for host in "${hosts[@]}"; do
    "$plistbuddy" -c "Add :$key:$index string applinks:$host" "$target_file"
    echo "Added applinks:$host"
    index=$((index + 1))
done
echo "Wrote $target_file"
