# Embedded Lucide catalog

Original SVG files from [lucide-icons/lucide](https://github.com/lucide-icons/lucide/tree/c2c28580b3009b8de62224c1fec6f78abb3e37d8/icons),
revision `c2c28580b3009b8de62224c1fec6f78abb3e37d8`.
The UI-only `fingerprint.svg` comes from the
[0.468.0 release](https://github.com/lucide-icons/lucide/blob/0.468.0/icons/fingerprint.svg).

Files are unmodified. Stable Taypeer key `home` maps to upstream `house.svg`;
`file-key-2` maps to `file-key.svg`; UI action key `trash-2` maps to `trash.svg`.
All other filenames match upstream.
`SHA256SUMS` records the exact vendored bytes. `LICENSE` includes the upstream
ISC license and MIT attribution for inherited Feather icons.

The shared Rust catalog embeds the SVGs and license; no network access is needed
to enumerate or render these keys. This small catalog can grow without changing
persisted keys. It is independent of platform UI libraries.
