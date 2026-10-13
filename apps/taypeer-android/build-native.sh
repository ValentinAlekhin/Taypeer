#!/bin/sh
# Build and generate from the same locked workspace dependency graph.
set -eu
cd "$(dirname "$0")"
ANDROID_PROJECT=$(pwd)
REPOSITORY=$(cd ../.. && pwd)
NATIVE_VARIANT=${1:-release}
case "$NATIVE_VARIANT" in
    debug) NATIVE_PROFILE=debug; NATIVE_FEATURES=android-test-support ;;
    release) NATIVE_PROFILE=release; NATIVE_FEATURES= ;;
    *) echo "Expected native variant debug or release" >&2; exit 1 ;;
esac
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
# The library and its generated bindings must expose the same variant capabilities.
set -- --locked -p taypeer-android
if [ "$NATIVE_VARIANT" = release ]; then set -- "$@" --release; fi
if [ -n "$NATIVE_FEATURES" ]; then set -- "$@" --features "$NATIVE_FEATURES"; fi
cargo build "$@" --lib
cargo run "$@" --features bindgen --bin taypeer-android-bindgen -- generate \
    --library "$CARGO_TARGET_DIR/$NATIVE_PROFILE/libtaypeer_android.$HOST_SUFFIX" --language kotlin \
    --config "$ANDROID_PROJECT/native/uniffi.toml" \
    --out-dir "$ANDROID_PROJECT/app/build/generated/uniffi/$NATIVE_VARIANT" --no-format
cargo build "$@" --lib --target aarch64-linux-android
mkdir -p "$ANDROID_PROJECT/app/build/generated/jniLibs/$NATIVE_VARIANT/arm64-v8a"
cp "$CARGO_TARGET_DIR/aarch64-linux-android/$NATIVE_PROFILE/libtaypeer_android.so" \
    "$ANDROID_PROJECT/app/build/generated/jniLibs/$NATIVE_VARIANT/arm64-v8a/"
