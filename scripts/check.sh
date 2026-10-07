#!/bin/sh
# Run from any directory. CI and local development share these checks.
set -eu
cd "$(dirname "$0")/.."

if [ "$#" -ne 0 ]; then
    echo "Usage: sh scripts/check.sh" >&2
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
cargo test --workspace --exclude taypeer --all-features --locked
cargo test -p taypeer --lib --all-features --locked
cargo test -p taypeer --features component-rendering --test components --locked
if [ "$(uname -s)" = Darwin ] || [ "$(uname -s)" = Linux ]; then
    cargo test -p taypeer --features ui-test-support --test ui --locked -- --test-threads=1
fi
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps --locked
cargo build --workspace --release --locked
cargo run --locked --quiet -p taypeer --features ui-test-support -- --smoke-test

echo "Native platform checks remain separate from the common project check."
