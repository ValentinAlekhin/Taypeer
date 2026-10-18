# Production parser harness

This isolated workspace exercises the actual parsers in `taypeer-document`,
`taypeer-storage` and `taypeer-trust`. Its lockfile starts from the production
dependency versions. It does not change the application workspace or its lockfile.

The deterministic smoke binary uses the frozen PUBLIC schema-one corpus and
validated signed examples extracted from it. It replays valid inputs, truncations,
bit flips and trailing bytes. Storage receives an immutable in-memory
`CiphertextIo` capability through the production archive/object APIs; no parser
or signature validator is replaced.

Two transient credential seeds use fixed PUBLIC keys and the fixture's exact
database/root/epoch: one enrolled author and one read-only record. The real
`LocalCredential::seal/open` codec supplies authenticated ciphertext with fresh
nonces; arbitrary credential decoding uses the fixed epoch key, without password
KDF work in fuzz iterations.

From the repository root, with the ordinary stable toolchain:

```sh
rtk cargo run --manifest-path fuzz/Cargo.toml --locked --bin corpus-smoke
rtk cargo clippy --manifest-path fuzz/Cargo.toml --locked --all-targets --features fuzzing -- -D warnings
```

`corpus-smoke` works without libFuzzer, nightly, C++ or system credentials. Its
optional output creates a new directory and never overwrites frozen fixtures:

```sh
rtk cargo run --manifest-path fuzz/Cargo.toml --locked --bin corpus-smoke -- --write-seeds /private/tmp/taypeer-parser-corpus
```

## Coverage-guided runs

Use cargo-fuzz 0.13.2, libfuzzer-sys 0.4.13 and nightly-2026-09-09. The optional
`fuzzing` feature enables three targets, with Address Sanitizer supplied by
cargo-fuzz:

- `document`: `Document::load`, document round trips and `OriginalChange::parse`.
- `storage`: archive, signed archive metadata, encrypted objects and keyed local credentials.
- `trust`: control chains pinned to the fixed PUBLIC root, signed manifests,
  envelopes, source proofs and invitations.

After exporting seeds, run each target with explicit input, time and memory budgets:

```sh
rtk proxy env RUSTUP_TOOLCHAIN=nightly-2026-09-09 cargo fuzz run --features fuzzing document /private/tmp/taypeer-parser-corpus/document -- -max_total_time=60 -max_len=1048576 -timeout=5 -rss_limit_mb=1024
rtk proxy env RUSTUP_TOOLCHAIN=nightly-2026-09-09 cargo fuzz run --features fuzzing storage /private/tmp/taypeer-parser-corpus/storage -- -max_total_time=60 -max_len=1048576 -timeout=5 -rss_limit_mb=1024
rtk proxy env RUSTUP_TOOLCHAIN=nightly-2026-09-09 cargo fuzz run --features fuzzing trust /private/tmp/taypeer-parser-corpus/trust -- -max_total_time=60 -max_len=1048576 -timeout=5 -rss_limit_mb=1024
```

The harness rejects inputs larger than 1 MiB. That budget does not prove bounded
decompression, memory use or execution time in production. Short smoke runs do
not close sustained fuzzing, large-document, KDF/AEAD, power-cut, native-memory
or independent security acceptance. No real database, password or credential
belongs in a fuzz corpus or crash artifact.
