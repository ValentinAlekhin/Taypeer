#!/bin/sh
# Local development bundle. No databases or preferences are bundled.
set -eu
cd "$(dirname "$0")/.."
if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
    echo "The macOS demo requires an Apple Silicon Mac." >&2
    exit 1
fi
signing_identity=${TAYPEER_SIGNING_IDENTITY:-Taypeer Local Development}
if ! /usr/bin/security find-identity -v -p codesigning | grep -Fq "\"$signing_identity\""; then
    echo "Signing identity not found: $signing_identity" >&2
    echo "Run sh scripts/setup-macos-signing.sh once, or set TAYPEER_SIGNING_IDENTITY to a certificate name." >&2
    exit 1
fi
cargo build --locked -p taypeer
sh scripts/build-macos-icon.sh
bundle="target/Taypeer.app"
mkdir -p "$bundle/Contents/MacOS"
mkdir -p "$bundle/Contents/Resources"
cp target/debug/taypeer "$bundle/Contents/MacOS/taypeer"
helper=$(target/debug/taypeer __platform-helper)
cp "$helper" "$bundle/Contents/MacOS/taypeer-platform"
cp apps/taypeer/Info.plist "$bundle/Contents/Info.plist"
cp target/macos/Taypeer.icns "$bundle/Contents/Resources/Taypeer.icns"
cp wireframes/assets/LICENSE-LUCIDE.txt "$bundle/Contents/Resources/LICENSE-LUCIDE.txt"
/usr/bin/codesign --force --sign "$signing_identity" --timestamp=none \
    --identifier dev.taypeer.demo.platform "$bundle/Contents/MacOS/taypeer-platform"
/usr/bin/codesign --force --sign "$signing_identity" --timestamp=none \
    --identifier dev.taypeer.demo "$bundle"
/usr/bin/codesign --verify --deep --strict "$bundle"
echo "Built $bundle"
