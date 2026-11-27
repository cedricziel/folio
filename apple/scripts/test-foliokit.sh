#!/usr/bin/env bash
# Runs the FolioKit test suite on a runtime that meets FolioKit's 27.2 floor.
#
#   test-foliokit.sh                use the host's macOS when it is 27.2 or newer, otherwise the iOS 27.2 simulator
#   test-foliokit.sh macos          fail unless the host itself meets the floor
#   test-foliokit.sh ios-simulator  always use the iOS 27.2 simulator
#
# Comparing two `Date`s resolves to Foundation's `Date.>=`, a symbol that only exists from 27.2 (see
# FolioKit/Sources/FolioKit/Sharing/FileLease.swift). A macOS test bundle built for 27.2 therefore cannot be loaded on
# a 27.0.x host: dyld reports "Symbol not found: _$s10Foundation4DateV2geoiySbAC_ACtFZ ... built for macOS 27.2 which
# is newer than running OS". The same Xcode still ships the iOS 27.2 simulator runtime, whose Foundation has the
# symbol, so the suite runs there until a 27.2 host is available. That run is real 27.2 coverage, but not macOS
# coverage, and the job summary says so instead of passing silently.
#
# GitHub's `xcode-27` image briefly shipped macOS 27.2, then went back to 27.0.1
# (https://github.com/actions/runner-images/issues/15334). This keeps the 27.2 floor without weakening it and without
# marking a dyld failure as success.
set -euo pipefail

minimum=27.2
scripts_dir="$(cd "$(dirname "$0")" && pwd)"
kit_dir="$(cd "$scripts_dir/../FolioKit" && pwd)"
mode="${1:-auto}"

at_least() {
    [ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n 1)" = "$2" ]
}

summary() {
    echo "$1"
    if [ -n "${GITHUB_STEP_SUMMARY:-}" ]; then
        printf '%s\n' "$1" >>"$GITHUB_STEP_SUMMARY"
    fi
}

host="$(sw_vers -productVersion)"
if [ "$mode" = auto ]; then
    if at_least "$host" "$minimum"; then mode=macos; else mode=ios-simulator; fi
fi

case "$mode" in
macos)
    if ! at_least "$host" "$minimum"; then
        echo "::error::macOS $host is older than FolioKit's $minimum floor, so the macOS tests cannot load. Run the ios-simulator destination instead." >&2
        exit 1
    fi
    echo "FolioKit tests: macOS $host, $minimum floor met (runner image ${ImageVersion:-unknown}, ${ImageOS:-unknown})"
    cd "$kit_dir"
    exec "$scripts_dir/test-watchdog.sh" xcrun swift test --no-parallel
    ;;
ios-simulator)
    selection="$(
        xcrun simctl list runtimes -j | python3 -c '
import json, sys

minimum = sys.argv[1]
preferred = "com.apple.CoreSimulator.SimDeviceType.iPhone-18-Pro"
for runtime in json.load(sys.stdin)["runtimes"]:
    if not runtime.get("isAvailable") or runtime["version"] != minimum:
        continue
    types = [d["identifier"] for d in runtime.get("supportedDeviceTypes", [])
             if "SimDeviceType.iPhone" in d["identifier"]]
    if not types:
        continue
    print(runtime["identifier"], preferred if preferred in types else types[0])
    break
' "$minimum"
    )" || true
    if [ -z "$selection" ]; then
        echo "::error::No available iOS $minimum simulator runtime in $(xcode-select -p | sed 's|/Contents/Developer$||'); install Xcode 27.2 or newer." >&2
        exit 1
    fi
    read -r runtime device_type <<<"$selection"

    device="$(xcrun simctl create "FolioKit-Tests-$$" "$device_type" "$runtime")"
    cleanup() {
        [ -n "${device:-}" ] || return 0
        xcrun simctl shutdown "$device" >/dev/null 2>&1 || true
        xcrun simctl delete "$device" >/dev/null 2>&1 || true
        device=""
    }
    # The watchdog stops the whole tree, this script included, so it also cleans up on the way out.
    trap cleanup EXIT
    trap 'exit 143' INT TERM
    xcrun simctl bootstatus "$device" -b

    # Announced before the run, so a failure below never reads as completed coverage.
    echo "FolioKit tests: iOS $minimum simulator ($runtime) on macOS $host (runner image ${ImageVersion:-unknown}, ${ImageOS:-unknown})"
    echo "::notice::macOS $host is below the $minimum floor; running FolioKit tests on the iOS $minimum simulator."

    # The simulator destination cross-compiles every package dependency, which takes far longer than the macOS suite.
    export TEST_WATCHDOG_LIMIT="${TEST_WATCHDOG_LIMIT:-2400}"

    cd "$kit_dir"
    "$scripts_dir/test-watchdog.sh" xcrun xcodebuild test \
        -scheme FolioKit-Package \
        -destination "platform=iOS Simulator,id=$device" \
        -derivedDataPath "$kit_dir/.build/ios-simulator" \
        -skipPackagePluginValidation -skipMacroValidation \
        -parallel-testing-enabled NO \
        CODE_SIGNING_ALLOWED=NO

    summary "### FolioKit tests ran on the iOS $minimum simulator"
    summary ""
    summary "The host is macOS $host (runner image ${ImageVersion:-unknown}, ${ImageOS:-unknown}), older than FolioKit's $minimum floor, so the macOS test bundle cannot be loaded."
    summary "This is real $minimum Foundation coverage on the iOS simulator, not macOS-runtime coverage."
    ;;
*)
    echo "usage: $0 [auto | macos | ios-simulator]" >&2
    exit 2
    ;;
esac
