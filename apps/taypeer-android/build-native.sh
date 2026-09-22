#!/bin/sh
# Build and generate from the same locked workspace dependency graph.
set -eu
cd "$(dirname "$0")"
ANDROID_PROJECT=$(pwd)
REPOSITORY=$(cd ../.. && pwd)
: "${ANDROID_HOME:=$ANDROID_PROJECT/.tools/android-sdk}"
export ANDROID_HOME
case "$(uname -s)" in
    Darwin) NDK_HOST=darwin-x86_64; HOST_SUFFIX=dylib ;;
    Linux) NDK_HOST=linux-x86_64; HOST_SUFFIX=so ;;
    *) echo "Unsupported build host" >&2; exit 1 ;;
esac
NDK_BIN="$ANDROID_HOME/ndk/27.2.12479018/toolchains/llvm/prebuilt/$NDK_HOST/bin"
test -x "$NDK_BIN/aarch64-linux-android31-clang" || {
    echo "Install NDK 27.2.12479018 in ANDROID_HOME first" >&2; exit 1;
}
: "${CARGO_TARGET_DIR:=$REPOSITORY/target}"
export CARGO_TARGET_DIR
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$NDK_BIN/aarch64-linux-android31-clang"
export CC_aarch64_linux_android="$CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER"
export AR_aarch64_linux_android="$NDK_BIN/llvm-ar"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"
cd "$REPOSITORY"
cargo build --locked -p taypeer-android --lib
cargo run --locked -p taypeer-android --features bindgen --bin taypeer-android-bindgen -- generate \
    --library "$CARGO_TARGET_DIR/debug/libtaypeer_android.$HOST_SUFFIX" --language kotlin \
    --config "$ANDROID_PROJECT/native/uniffi.toml" \
    --out-dir "$ANDROID_PROJECT/app/build/generated/uniffi" --no-format
cargo build --locked -p taypeer-android --lib --target aarch64-linux-android
mkdir -p "$ANDROID_PROJECT/app/build/generated/jniLibs/arm64-v8a"
cp "$CARGO_TARGET_DIR/aarch64-linux-android/debug/libtaypeer_android.so" \
    "$ANDROID_PROJECT/app/build/generated/jniLibs/arm64-v8a/"
