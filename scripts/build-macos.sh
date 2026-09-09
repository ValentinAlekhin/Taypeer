#!/bin/sh
# Local, unsigned demonstration bundle. No databases or preferences are bundled.
set -eu
cd "$(dirname "$0")/.."
if [ "$(uname -s)" != Darwin ] || [ "$(uname -m)" != arm64 ]; then
    echo "The macOS demo requires an Apple Silicon Mac." >&2
    exit 1
fi
cargo build --locked -p taypeer
sh scripts/build-macos-icon.sh
bundle="target/Taypeer Demo.app"
mkdir -p "$bundle/Contents/MacOS"
mkdir -p "$bundle/Contents/Resources"
cp target/debug/taypeer "$bundle/Contents/MacOS/taypeer"
cp apps/taypeer/Info.plist "$bundle/Contents/Info.plist"
cp target/macos/Taypeer.icns "$bundle/Contents/Resources/Taypeer.icns"
cp wireframes/assets/LICENSE-LUCIDE.txt "$bundle/Contents/Resources/LICENSE-LUCIDE.txt"
echo "Built $bundle"
