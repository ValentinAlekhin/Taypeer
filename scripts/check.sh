#!/bin/sh
# Run from any directory. CI and local development share these checks.
set -eu
cd "$(dirname "$0")/.."

case "${1-}" in
    "") native_keychain=0 ;;
    --native-keychain) native_keychain=1 ;;
    *) echo "Usage: sh scripts/check.sh [--native-keychain]" >&2; exit 2 ;;
esac
if [ "$#" -gt 1 ]; then
    echo "Usage: sh scripts/check.sh [--native-keychain]" >&2
    exit 2
fi

if ! command -v pnpm >/dev/null 2>&1 || [ ! -x node_modules/.bin/likec4 ]; then
    echo "Architecture tooling missing. Install pnpm 10.17.1, then run pnpm install --frozen-lockfile." >&2
    exit 1
fi
pnpm arch:check

cargo fmt --all -- --check
cargo check-all
cargo lint
cargo test-all
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo build --workspace --release --locked
cargo run --locked --quiet -p taypeer -- --smoke-test

if [ "$native_keychain" -eq 1 ]; then
    echo "Explicit native tests: macOS Keychain may request access. Do not rebuild CLI concurrently."
    cargo test --locked -p taypeer-cli --tests -- --ignored --test-threads=1
else
    echo "Native Keychain tests skipped; opt in with --native-keychain."
fi
