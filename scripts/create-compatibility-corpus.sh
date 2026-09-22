#!/bin/sh
# Explicit PUBLIC-only generator. Never invoked by check.sh or ordinary tests.
set -eu
if [ "$#" -ne 1 ]; then
    echo "Usage: sh scripts/create-compatibility-corpus.sh NEW_OUTPUT_DIRECTORY" >&2
    exit 2
fi
case "$1" in
    /*) output=$1 ;;
    *) output="$PWD/$1" ;;
esac
if [ -e "$output" ]; then
    echo "Output must be a new directory; frozen fixtures are never overwritten." >&2
    exit 2
fi
cd "$(dirname "$0")/.."
export TAYPEER_CORPUS_OUTPUT="$output"
cargo test --locked -p taypeer-services --lib managed::tests::corpus::generate_development_corpus -- --ignored --exact
cargo test --locked -p taypeer-document --lib tests::corpus::generate_extension_corpus -- --ignored --exact
(cd "$output" && shasum -a 256 -- *.taypeer *.draft *.json *.automerge > SHA256SUMS)
