#!/bin/sh
# Run from any directory; dependencies/SDK licenses must already be installed.
set -eu
spike_root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
: "${ANDROID_SDK_ROOT:?Set ANDROID_SDK_ROOT to the installed SDK}"
: "${JAVA_HOME:?Set JAVA_HOME to JDK 17}"
ndk_version=${TAYPEER_NDK_VERSION:-27.2.12479018}
host_tag=${TAYPEER_NDK_HOST:-darwin-x86_64}
toolchain="$ANDROID_SDK_ROOT/ndk/$ndk_version/toolchains/llvm/prebuilt/$host_tag/bin"
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$toolchain/aarch64-linux-android31-clang"
export CC_aarch64_linux_android="$toolchain/aarch64-linux-android31-clang"
export AR_aarch64_linux_android="$toolchain/llvm-ar"
export CARGO_TARGET_DIR="$spike_root/target"
harness="$CARGO_TARGET_DIR/android-harness"
mkdir -p "$harness"
cargo test --manifest-path "$spike_root/Cargo.toml" --target aarch64-linux-android \
  --features android-jni --release --no-run --locked --message-format=json > "$harness/build.json"
"$JAVA_HOME/bin/javac" -d "$harness" "$spike_root/scripts/TaypeerNative.java"
"$ANDROID_SDK_ROOT/build-tools/35.0.0/d8" --min-api 31 --output "$harness" "$harness/TaypeerNative.class"
printf 'Android harness built: %s\n' "$harness"
