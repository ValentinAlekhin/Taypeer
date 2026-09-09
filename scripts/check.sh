#!/bin/sh
# Run from any directory. CI and local development share these checks.
set -eu
cd "$(dirname "$0")/.."

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
cargo run --locked --quiet -p taypeer
