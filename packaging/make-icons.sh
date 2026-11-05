#!/bin/bash
# Build all app icons from the generated fox-tail art (packaging/icon_src.png, made with Codex CLI).
#  - icon_1024.png : full-bleed square, mirrored so the tail reads as a "Z" (macOS applies its own mask;
#                    a pre-shaped icon with transparent margins gets shrunk into a grey tile on macOS 26+)
#  - AppIcon.icns  : macOS
#  - tailzip.ico   : Windows (rounded corners, transparent outside)
set -e
PK=$(cd "$(dirname "$0")" && pwd)
uvx --quiet --with pillow python - "$PK" <<'EOF'
import sys
from PIL import Image, ImageDraw, ImageOps
pk = sys.argv[1]
src = Image.open(f"{pk}/icon_src.png").convert("RGB")
# inner square inside the generator's own rounded corners, mirrored left-right
sq = ImageOps.mirror(src.crop((102, 110, 912, 920))).resize((1024, 1024), Image.LANCZOS)
sq.save(f"{pk}/icon_1024.png")
# Windows: rounded square with transparent corners
big = 1024 * 4
m = Image.new("L", (big, big), 0)
ImageDraw.Draw(m).rounded_rectangle((0, 0, big - 1, big - 1), radius=int(big * 0.18), fill=255)
win = sq.convert("RGBA"); win.putalpha(m.resize((1024, 1024), Image.LANCZOS))
win.save(f"{pk}/icon_win_1024.png")
win.save(f"{pk}/tailzip.ico", sizes=[(16, 16), (24, 24), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)])
EOF
IS=$PK/AppIcon.iconset; rm -rf "$IS"; mkdir -p "$IS"
for s in 16 32 128 256 512; do
  sips -z $s $s "$PK/icon_1024.png" --out "$IS/icon_${s}x${s}.png" >/dev/null
  d=$((s*2)); sips -z $d $d "$PK/icon_1024.png" --out "$IS/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$IS" -o "$PK/AppIcon.icns"
rm -rf "$IS"
ls -la "$PK/AppIcon.icns" "$PK/tailzip.ico" "$PK/icon_1024.png"
