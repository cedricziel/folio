#!/usr/bin/env bash
# Checks that the generated Folio project resolves the identifiers it should.
#
#   check-build-settings.sh            official build: no Local.xcconfig, today's identifiers
#   check-build-settings.sh --local    your Config/Local.xcconfig: the values it sets
#   check-build-settings.sh --example  Config/Local.xcconfig.example copied in as Local.xcconfig
#
# Needs xcodegen and Xcode. Regenerates apple/Folio/Folio.xcodeproj.
set -euo pipefail

folio_dir="$(cd "$(dirname "$0")/../Folio" && pwd)"
local_config="$folio_dir/Config/Local.xcconfig"
mode="${1:-official}"
failures=0

# Prints KEY's value from Local.xcconfig, or DEFAULT when the file does not set it.
local_value() {
    local value
    value="$(sed -n "s/^[[:space:]]*$1[[:space:]]*=[[:space:]]*\\(.*\\)$/\\1/p" "$local_config" | tail -n 1)"
    if grep -q "^[[:space:]]*$1[[:space:]]*=" "$local_config"; then
        printf '%s' "${value%"${value##*[![:space:]]}"}"
    else
        printf '%s' "$2"
    fi
}

case "$mode" in
official)
    if [ -e "$local_config" ]; then
        echo "error: $local_config exists; check it with --local, or move it away for the official check." >&2
        exit 2
    fi
    ;;
--example)
    if [ -e "$local_config" ]; then
        echo "error: $local_config exists; move it away before checking the example." >&2
        exit 2
    fi
    cp "$folio_dir/Config/Local.xcconfig.example" "$local_config"
    trap 'rm -f "$local_config"' EXIT
    ;;
--local)
    if [ ! -e "$local_config" ]; then
        echo "error: $local_config does not exist; copy Config/Local.xcconfig.example first." >&2
        exit 2
    fi
    ;;
*)
    echo "usage: $0 [--local | --example]" >&2
    exit 2
    ;;
esac

team=WPW3QSLZ2F
bundle=com.cedricziel.folio
app_group=group.com.cedricziel.folio
app_entitlements=Resources/Folio.entitlements
sign_style=Manual
identity="Apple Distribution"
if [ -e "$local_config" ]; then
    team="$(local_value FOLIO_DEVELOPMENT_TEAM "$team")"
    bundle="$(local_value FOLIO_BUNDLE_ID "$bundle")"
    app_group="$(local_value FOLIO_APP_GROUP "$app_group")"
    app_entitlements="$(local_value FOLIO_APP_ENTITLEMENTS "$app_entitlements")"
    sign_style="$(local_value FOLIO_RELEASE_SIGN_STYLE "$sign_style")"
    identity="$(local_value FOLIO_RELEASE_IDENTITY "$identity")"
fi

# The Release profile for a target and SDK: Local.xcconfig's value, or the official match profile.
profile_for() {
    local key="FOLIO_PROFILE_$1_$2" official="match AppStore $bundle$3"
    [ "$2" = MACOS ] && official="$official macos"
    if [ -e "$local_config" ]; then local_value "$key" "$official"; else printf '%s' "$official"; fi
}

(cd "$folio_dir" && xcodegen generate --quiet)

# Prints KEY's value from `xcodebuild -showBuildSettings` output on stdin; empty when unset.
setting() {
    awk -v key="$1" '
        { sub(/^ +/, "") }
        $0 == key " =" { exit }
        index($0, key " = ") == 1 { print substr($0, length(key) + 4); exit }
    '
}

expect() {
    local label="$1" actual="$2" expected="$3"
    if [ "$actual" = "$expected" ]; then
        printf 'ok    %s = %s\n' "$label" "$actual"
    else
        printf 'FAIL  %s = "%s", expected "%s"\n' "$label" "$actual" "$expected"
        failures=$((failures + 1))
    fi
}

# Handoff needs every activity type the app advertises or accepts declared in its Info.plist.
activity_types="$(plutil -extract NSUserActivityTypes json -o - "$folio_dir/Resources/Folio-Info.plist" || true)"
expect "Folio-Info.plist NSUserActivityTypes" "$activity_types" \
    '["com.cedricziel.folio.document","com.cedricziel.folio.library","com.cedricziel.folio.saved-view"]'

# The App Store profile has no Associated Domains capability; the key would break signing.
if [ "$mode" = official ] &&
    grep -q com.apple.developer.associated-domains "$folio_dir/Resources/Folio.entitlements"; then
    echo "FAIL  Resources/Folio.entitlements contains com.apple.developer.associated-domains"
    failures=$((failures + 1))
fi

# target  bundle-id-suffix  entitlements  profile-key
targets=(
    "Folio||$app_entitlements|APP"
    "FolioShare|.share|Resources/FolioShare.entitlements|SHARE"
    "FolioFileProvider|.fileprovider|Resources/FolioFileProvider.entitlements|FILEPROVIDER"
)

for entry in "${targets[@]}"; do
    IFS='|' read -r target suffix entitlements profile_key <<<"$entry"
    for sdk in iphoneos macosx; do
        for configuration in Debug Release; do
            settings="$(cd "$folio_dir" && xcodebuild -showBuildSettings -project Folio.xcodeproj \
                -target "$target" -configuration "$configuration" -sdk "$sdk")"
            label="$target $configuration $sdk"
            expect "$label PRODUCT_BUNDLE_IDENTIFIER" "$(setting PRODUCT_BUNDLE_IDENTIFIER <<<"$settings")" \
                "$bundle$suffix"
            expect "$label DEVELOPMENT_TEAM" "$(setting DEVELOPMENT_TEAM <<<"$settings")" "$team"
            expect "$label CODE_SIGN_ENTITLEMENTS" "$(setting CODE_SIGN_ENTITLEMENTS <<<"$settings")" "$entitlements"
            expect "$label FOLIO_APP_GROUP" "$(setting FOLIO_APP_GROUP <<<"$settings")" "$app_group"
            [ "$configuration" = Release ] || continue
            if [ "$sdk" = macosx ]; then platform=MACOS; else platform=IOS; fi
            profile="$(profile_for "$profile_key" "$platform" "$suffix")"
            expect "$label CODE_SIGN_STYLE" "$(setting CODE_SIGN_STYLE <<<"$settings")" "$sign_style"
            expect "$label CODE_SIGN_IDENTITY" "$(setting CODE_SIGN_IDENTITY <<<"$settings")" "$identity"
            expect "$label PROVISIONING_PROFILE_SPECIFIER" \
                "$(setting PROVISIONING_PROFILE_SPECIFIER <<<"$settings")" "$profile"
        done
    done
done

if [ "$failures" -gt 0 ]; then
    echo "$failures build setting(s) differ from the $mode configuration." >&2
    exit 1
fi
echo "All build settings match the $mode configuration."
