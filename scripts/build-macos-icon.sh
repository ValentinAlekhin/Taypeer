#!/bin/sh
# Package the source artwork with the standard macOS tools; output stays in target.
set -eu
cd "$(dirname "$0")/.."

source_icon="wireframes/assets/taypeer-app-icon-v1.png"
iconset="target/macos/Taypeer.iconset"
mkdir -p "$iconset"

for size in 16 32 128 256 512; do
    sips -z "$size" "$size" "$source_icon" \
        --out "$iconset/icon_${size}x${size}.png" >/dev/null
    retina_size=$((size * 2))
    sips -z "$retina_size" "$retina_size" "$source_icon" \
        --out "$iconset/icon_${size}x${size}@2x.png" >/dev/null
done

iconutil -c icns "$iconset" -o target/macos/Taypeer.icns
