#!/bin/sh
# Build the isolated synthetic probe; no SDK licenses are accepted by this script.
set -eu
cd "$(dirname "$0")"
PROBE_ROOT=$(pwd)
: "${ANDROID_HOME:=$PROBE_ROOT/.tools/android-sdk}"
: "${GRADLE_USER_HOME:=$PROBE_ROOT/.tools/gradle-cache}"
export ANDROID_HOME GRADLE_USER_HOME
PROBE_TOOLCHAIN="$ANDROID_HOME/ndk/27.2.12479018/toolchains/llvm/prebuilt/darwin-x86_64/bin"
if [ "$(uname -s)" = Linux ]; then
    PROBE_TOOLCHAIN="$ANDROID_HOME/ndk/27.2.12479018/toolchains/llvm/prebuilt/linux-x86_64/bin"
fi
test -x "$PROBE_TOOLCHAIN/aarch64-linux-android31-clang"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$PROBE_TOOLCHAIN/aarch64-linux-android31-clang"
export CC_aarch64_linux_android="$CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER"
export AR_aarch64_linux_android="$PROBE_TOOLCHAIN/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"
cargo +1.97.1 test --locked
cargo +1.97.1 build --locked --lib
PROBE_HOST_LIBRARY=target/debug/libtaypeer_ui_probe.dylib
if [ "$(uname -s)" = Linux ]; then PROBE_HOST_LIBRARY=target/debug/libtaypeer_ui_probe.so; fi
cargo +1.97.1 run --locked --features bindgen --bin uniffi-bindgen -- generate \
    --library "$PROBE_HOST_LIBRARY" --language kotlin --config uniffi.toml \
    --out-dir app/build/generated/uniffi --no-format
cargo +1.97.1 build --locked --lib --target aarch64-linux-android
mkdir -p app/src/main/jniLibs/arm64-v8a
cp target/aarch64-linux-android/debug/libtaypeer_ui_probe.so app/src/main/jniLibs/arm64-v8a/
./gradlew --no-daemon :app:assembleDebug :app:assembleDebugAndroidTest :app:lintDebug "$@"
python3 verify-apk.py app/build/outputs/apk/debug/app-debug.apk
"$ANDROID_HOME/build-tools/35.0.0/zipalign" -c -P 16 4 app/build/outputs/apk/debug/app-debug.apk
