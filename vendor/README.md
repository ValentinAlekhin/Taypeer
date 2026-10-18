# RustCrypto compatibility patch

`chacha20/` is the published RustCrypto `chacha20 0.9.1` crate with a small
zeroization patch. The algorithm, public API, round functions and test vectors
are unchanged. Both the root and isolated fuzz workspaces select it with
`[patch.crates-io]`; the Nix source filter includes this directory. It is excluded
from workspace membership so upstream code keeps its own feature/lint policy.

## Provenance

- Published archive: <https://static.crates.io/crates/chacha20/chacha20-0.9.1.crate>.
- SHA-256: `c3613f74bd2eac03dad61bd53dbe620703d4371614fe0bc3b9f04dd36fe4e818`.
  Verified against the registry checksum from the original Taypeer lockfile
  before extracting the archive.
- Upstream revision from `.cargo_vcs_info.json`:
  [`b5d39c45c45df97335f4b6effb853cde97810de1`](https://github.com/RustCrypto/stream-ciphers/tree/b5d39c45c45df97335f4b6effb853cde97810de1/chacha20).
- Upstream `LICENSE-APACHE` and `LICENSE-MIT` are retained, along with all
  published sources, tests, vectors, benches and documentation.

The only upstream file changes are `src/xchacha.rs`, `Cargo.toml` and
`Cargo.toml.orig`. A standalone `Cargo.lock` pins the upstream test dependencies;
production and fuzz builds use their respective workspace lockfiles.

## Patch scope

With feature `zeroize`, `XChaChaCore::new` owns the HChaCha subkey through
`Zeroizing<GenericArray<u8, U32>>`; `hchacha` owns its working state through
`Zeroizing<[u32; 16]>`. Drop wipes those logical buffers on normal return and
unwinding. The manifests enable the existing `generic-array/zeroize`
implementation, needed for the subkey owner. With the feature disabled, the
original code path remains. The patch adds no allocation or `std` requirement.

This does not guarantee removal of all compiler-generated moves, register/spill
copies, or memory after abort, swap or core dumps. Callers of the public
`hchacha` function still own and must protect its returned key. The wider memory
boundary remains tracked in [the security review](../docs/security-review.md).

Remove both workspace overrides when a compatible upstream release provides
equivalent cleanup, then repeat the vectors, storage tests and frozen corpus
checks. A dependency upgrade alone is not evidence of cleanup.

## Targeted verification

Run from the repository root. Both feature modes use the same target directory
and the repository's small development-profile settings:

```sh
rtk cargo test --locked --manifest-path vendor/chacha20/Cargo.toml --target-dir target --config profile.dev.debug=0 --config profile.dev.incremental=false --no-default-features --lib --test mod
rtk cargo test --locked --manifest-path vendor/chacha20/Cargo.toml --target-dir target --config profile.dev.debug=0 --config profile.dev.incremental=false --no-default-features --features zeroize --lib --test mod
rtk cargo test --locked -p taypeer-storage --lib
rtk cargo test --locked -p taypeer-services --lib managed::tests::corpus
```

The unchanged upstream tests include HChaCha known-answer, RFC ChaCha vectors,
XChaCha encryption/keystream vectors and seek behavior. These and the frozen
application corpus verify compatibility; ownership and the `Zeroizing` Drop
implementation establish source-level cleanup, not a physical-memory wipe test.
