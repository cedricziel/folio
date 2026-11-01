#!/usr/bin/env bash
# Regenerates Folio/Resources/Assets.xcassets/AppIcon.appiconset from static/icons/icon.svg.
# Needs: rsvg-convert, magick (brew install librsvg imagemagick).
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
out="$root/apple/Folio/Resources/Assets.xcassets/AppIcon.appiconset"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

paper="#f6f6f2"
archive="#1f6b5c"
mark='<rect x="97.3" y="122.9" width="143.4" height="133.1" rx="25.6" fill="'$archive'"/>
<rect x="97.3" y="194.6" width="317.4" height="215.0" rx="30.7" fill="'$archive'"/>'

# iOS: full-bleed square, no alpha, no baked corners (the system masks it).
cat > "$work/ios.svg" <<SVG
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 512 512"><rect width="512" height="512" fill="$paper"/>$mark</svg>
SVG
rsvg-convert -w 1024 -h 1024 "$work/ios.svg" -o "$work/ios.png"
magick "$work/ios.png" -background "$paper" -alpha remove -alpha off -define png:color-type=2 "$out/icon-1024.png"

# macOS: 824pt rounded rect centred on the 1024 grid, with a soft drop shadow.
cat > "$work/mac.svg" <<SVG
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1024 1024">
  <defs><filter id="s" x="-10%" y="-10%" width="120%" height="125%">
    <feDropShadow dx="0" dy="10" stdDeviation="12" flood-color="#000" flood-opacity="0.25"/>
  </filter></defs>
  <g filter="url(#s)"><rect x="100" y="100" width="824" height="824" rx="185.4" fill="$paper"/></g>
  <g transform="translate(100 100) scale(1.609375)">$mark</g>
</svg>
SVG
rsvg-convert -w 1024 -h 1024 "$work/mac.svg" -o "$work/mac-1024.png"

for size in 16 32 64 128 256 512 1024; do
  magick "$work/mac-1024.png" -filter Lanczos -resize "${size}x${size}" "$out/mac-$size.png"
done

cat > "$out/Contents.json" <<'JSON'
{
  "images" : [
    { "filename" : "icon-1024.png", "idiom" : "universal", "platform" : "ios", "size" : "1024x1024" },
    { "filename" : "mac-16.png", "idiom" : "mac", "scale" : "1x", "size" : "16x16" },
    { "filename" : "mac-32.png", "idiom" : "mac", "scale" : "2x", "size" : "16x16" },
    { "filename" : "mac-32.png", "idiom" : "mac", "scale" : "1x", "size" : "32x32" },
    { "filename" : "mac-64.png", "idiom" : "mac", "scale" : "2x", "size" : "32x32" },
    { "filename" : "mac-128.png", "idiom" : "mac", "scale" : "1x", "size" : "128x128" },
    { "filename" : "mac-256.png", "idiom" : "mac", "scale" : "2x", "size" : "128x128" },
    { "filename" : "mac-256.png", "idiom" : "mac", "scale" : "1x", "size" : "256x256" },
    { "filename" : "mac-512.png", "idiom" : "mac", "scale" : "2x", "size" : "256x256" },
    { "filename" : "mac-512.png", "idiom" : "mac", "scale" : "1x", "size" : "512x512" },
    { "filename" : "mac-1024.png", "idiom" : "mac", "scale" : "2x", "size" : "512x512" }
  ],
  "info" : { "author" : "xcode", "version" : 1 }
}
JSON
