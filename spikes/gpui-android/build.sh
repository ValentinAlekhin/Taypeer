#!/bin/sh
set -eu
cd "$(dirname "$0")"
cargo +1.97.1 build --locked
mkdir -p target/TaypeerDesktopProbe.app/Contents/MacOS
cp target/debug/taypeer-gpui-probe target/TaypeerDesktopProbe.app/Contents/MacOS/TaypeerDesktopProbe
cp Info.plist target/TaypeerDesktopProbe.app/Contents/Info.plist
