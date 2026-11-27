#!/usr/bin/env bash
# Runs the macOS-only app test bundles, FolioAuthTests and FolioProviderTests.
#
# They link FolioKit and so reference Foundation's `Date.>=`, which only exists from macOS 27.2 (see
# test-foliokit.sh). On an older host the bundles cannot be loaded at all: dyld reports "Symbol not found:
# _$s10Foundation4DateV2geoiySbAC_ACtFZ ... built for macOS 27.2 which is newer than running OS". Rather than turn that
# into a red job nobody can fix, the run is skipped and the job summary says these bundles did not run — a coverage gap
# to close on a 27.2 host, not a pass. On a 27.2 host they run as usual and any failure fails the lane.
set -euo pipefail

minimum=27.2
scripts_dir="$(cd "$(dirname "$0")" && pwd)"
folio_dir="$(cd "$scripts_dir/../Folio" && pwd)"
host="$(sw_vers -productVersion)"

at_least() {
    [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n 1)" = "$2" ]
}

if ! at_least "$host" "$minimum"; then
    message="FolioAuthTests and FolioProviderTests did not run: they need macOS $minimum and the host is $host."
    echo "::warning::$message"
    if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
        {
            echo "### macOS-only app tests skipped"
            echo
            echo "$message"
            echo "They link FolioKit, whose \`Date.>=\` is a 27.2 Foundation symbol, so the bundles cannot load here."
            echo "This is a coverage gap until the runner image returns to 27.2, not a pass."
        } >>"$GITHUB_STEP_SUMMARY"
    fi
    exit 0
fi

cd "$folio_dir"
exec xcrun xcodebuild test -project Folio.xcodeproj -scheme Folio \
    -destination 'platform=macOS' \
    -skipPackagePluginValidation -skipMacroValidation \
    -only-testing:FolioAuthTests -only-testing:FolioProviderTests \
    CODE_SIGNING_ALLOWED=NO
