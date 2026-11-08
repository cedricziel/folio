#!/usr/bin/env bash
# Checks that the generated Folio project resolves the identifiers it should.
#
#   check-build-settings.sh            official build: no Local.xcconfig, today's identifiers
#   check-build-settings.sh --example  Config/Local.xcconfig.example copied in as Local.xcconfig
#
# Needs xcodegen and Xcode. Regenerates apple/Folio/Folio.xcodeproj.
set -euo pipefail

folio_dir="$(cd "$(dirname "$0")/../Folio" && pwd)"
local_config="$folio_dir/Config/Local.xcconfig"
mode="${1:-official}"
failures=0

case "$mode" in
official)
    if [ -e "$local_config" ]; then
        echo "error: $local_config exists; the official check needs it absent." >&2
        exit 2
    fi
    team=WPW3QSLZ2F
    bundle=com.cedricziel.folio
    app_group=group.com.cedricziel.folio
    app_entitlements=Resources/Folio.entitlements
    sign_style=Manual
    identity="Apple Distribution"
    ;;
--example)
    if [ -e "$local_config" ]; then
        echo "error: $local_config exists; move it away before checking the example." >&2
        exit 2
    fi
    cp "$folio_dir/Config/Local.xcconfig.example" "$local_config"
    trap 'rm -f "$local_config"' EXIT
    team=ABCDE12345
    bundle=com.example.folio
    app_group=group.com.example.folio
    app_entitlements=Resources/Folio-Local.entitlements
    sign_style=Automatic
    identity="Apple Development"
    ;;
*)
    echo "usage: $0 [--example]" >&2
    exit 2
    ;;
esac

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

# target  bundle-id-suffix  entitlements  profile-key-suffix
targets=(
    "Folio||$app_entitlements|"
    "FolioShare|.share|Resources/FolioShare.entitlements|.share"
    "FolioFileProvider|.fileprovider|Resources/FolioFileProvider.entitlements|.fileprovider"
)

for entry in "${targets[@]}"; do
    IFS='|' read -r target suffix entitlements profile_suffix <<<"$entry"
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
            if [ "$mode" = official ]; then
                profile="match AppStore $bundle$profile_suffix"
                [ "$sdk" = macosx ] && profile="$profile macos"
            else
                profile=""
            fi
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
