#!/usr/bin/env bash
# assets/icon/build.sh: renders merlin's icon files from the SVGs here.
# Needs inkscape, ImageMagick (magick) and Python with Pillow. Run it after
# changing an SVG, and commit what it writes.
#
#   png/merlin-<size>.png  16 to 1024; the small drawing at 32 and under
#   merlin.ico             Windows: 16, 24, 32, 48, 64, 128, 256
#   merlin.icns            macOS, from merlin-macos.svg
set -euo pipefail
cd "$(dirname "$0")"
mkdir -p png mac
render() { inkscape "$1" --export-type=png --export-filename="$2" -w "$3" -h "$3" >/dev/null 2>&1; }
for size in 16 24 32; do render merlin-small.svg "png/merlin-$size.png" "$size"; done
for size in 48 64 128 256 512 1024; do render merlin.svg "png/merlin-$size.png" "$size"; done
for size in 16 32 64 128 256 512 1024; do render merlin-macos.svg "mac/$size.png" "$size"; done
magick png/merlin-{16,24,32,48,64,128,256}.png merlin.ico
python3 - <<'PY'
from PIL import Image
big = Image.open("mac/1024.png")
big.save("merlin.icns", sizes=[(s, s) for s in (16, 32, 64, 128, 256, 512, 1024)])
PY
rm -rf mac
echo "Wrote png/, merlin.ico and merlin.icns"
