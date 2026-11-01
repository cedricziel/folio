#!/usr/bin/env bash
# Renders a PDF page through the ImageMagick `convert` fallback inside a built image, with the
# same arguments as crates/folio-infra/src/thumbnail/mod.rs, and checks that other risky coders
# stay blocked by the policy.
set -euo pipefail

image="${1:?usage: $0 <image>}"

docker run --rm --entrypoint sh "$image" -c '
set -eu
[ "$(id -u)" = 10001 ]
cd "$(mktemp -d)"
cat > in.pdf <<PDF
%PDF-1.4
1 0 obj<</Type/Catalog/Pages 2 0 R>>endobj
2 0 obj<</Type/Pages/Kids[3 0 R]/Count 1>>endobj
3 0 obj<</Type/Page/Parent 2 0 R/MediaBox[0 0 200 200]/Contents 4 0 R>>endobj
4 0 obj<</Length 35>>stream
0 0 0 rg 20 20 160 160 re f
endstream
endobj
trailer<</Root 1 0 R/Size 5>>
%%EOF
PDF
convert -density 150 "in.pdf[0]" -resize 100x100 -quality 85 out.jpg
[ "$(identify -format %m out.jpg)" = JPEG ]
if convert -size 8x8 xc:red out.ps 2>/dev/null; then
  echo "PS coder should stay blocked" >&2
  exit 1
fi
echo "PDF fallback OK"
'
