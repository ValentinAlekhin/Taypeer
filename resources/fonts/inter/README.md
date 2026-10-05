# Embedded Inter

Unmodified static TrueType faces from the official
[Inter 4.1 release](https://github.com/rsms/inter/releases/tag/v4.1), archive
`Inter-4.1.zip`, directory `extras/ttf/`.

The desktop UI embeds Regular (400), Medium (500), SemiBold (600) and Bold (700).
The internal family name is `Inter`. [LICENSE.txt](LICENSE.txt) contains the
upstream SIL Open Font License 1.1; `SHA256SUMS` records the exact vendored bytes.
macOS and Nix packages include the license alongside the application.

[Registration](../../../apps/taypeer/crates/taypeer-ui/src/fonts.rs) runs before
the first window in both the application and headless UI sessions. The product
theme selects Inter again after theme or zoom changes. No OS font installation
or network request is needed. The monospace family for secrets and codes is
selected separately.
