# Иконка приложения Taypeer

[taypeer-app-icon-v1.png](taypeer-app-icon-v1.png) — сгенерированный исходный
PNG, 1254 × 1254, с альфа-каналом. Два серебристых звена и замочная скважина
на графитовой скруглённой плитке обозначают связь устройств и хранение паролей.
Палитра опирается на текущие макеты в `wireframes/source/compact.js`.

Создано встроенным инструментом `image_gen` (без CLI/API fallback).
Изображение просмотрено; размеры и наличие альфа-канала проверены через `sips`.
Подключён к macOS `.app`: [скрипт упаковки](../../scripts/build-macos-icon.sh)
создаёт `.icns` с размерами 16, 32, 128, 256 и 512 pt в масштабах 1× и 2×.
[Сборка приложения](../../scripts/build-macos.sh) включает его в ресурсы;
[Info.plist](../../apps/taypeer/Info.plist) задаёт иконку для macOS.
Производные файлы находятся в `target/` и не коммитятся. Android пока не подключён.
FIG не изменён.

## Промпт

```text
Use case: logo-brand.
Asset type: a single finished application icon for Taypeer, a local-first personal password manager that syncs between the owner's own devices. Generate the actual icon asset, not a presentation mockup.
Primary request: a beautiful, restrained, highly recognizable graphite and silver app icon. One bold central emblem: two substantial interlocking rounded links, composed into a compact upright vault/lock silhouette, with one clean dark keyhole cutout in the center. The two connected pieces suggest peer-to-peer trust; the keyhole suggests private password storage. Keep the silhouette extremely simple and optically balanced, readable at small Dock sizes.
Style/medium: precise geometric icon design with subtle satin-metal relief, broad clean surfaces, softly rounded edges, minimal depth. Calm professional desktop software aesthetic. Fine controlled upper-left light, no dramatic reflections.
Color palette: near-black graphite #181818 tile, restrained #303030 edges, silver/off-white #E6E6E6 emblem. Strictly monochrome.
Composition/framing: square 1024x1024 canvas, a single front-facing centered rounded-square app tile occupying about 88% of the canvas, generous rounded corners. Emblem centered and occupying about 58% of tile width with ample breathing room. Actual transparent alpha outside the rounded-square tile; no floor, no backdrop, no mockup environment. Crisp edges, symmetrical visual weight.
Constraints: one icon only. No words, letters, wordmark, watermark, tiny ornament, screws, circuitry, extra badges, neon, purple, clouds, glass, perspective tilt, or multiple variants. A restrained memorable symbol suited to a compact monochrome macOS utility.
```
