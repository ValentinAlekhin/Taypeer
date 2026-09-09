# Референсы

Локальные checkout в `ref/` исключены из Git и сборки. Ни один из них не является
зависимостью Taypeer. Таблица фиксирует HEAD имеющихся копий на момент подготовки;
локальные изменения этих копий не переносятся автоматически.

| Каталог | Upstream | HEAD |
| --- | --- | --- |
| `ref/zedra` | https://github.com/tanlethanh/zedra | `35f86b4b4b33c91365e99bde399374d6b64a3bc2` |
| `ref/iroh-examples` | https://github.com/n0-computer/iroh-examples | `6a8cfcdccc6a633c5608cb25e776cb52fd509dd3` |
| `ref/keepassxc` | https://github.com/keepassxreboot/keepassxc | `4243717a0b398456ffbdeb717983bd685f583ac5` |

Чтобы воспроизвести исходную ревизию, клонировать указанный upstream в его
каталог и выполнить `git checkout --detach <HEAD>`. Для обычной сборки это не нужно.

- Zedra — reference исходной гипотезы мобильного GPUI и платформенных интеграций.
  Совместимость Kit с Android не подтверждена; текущее решение использует
  Kotlin + Compose на Android и сохраняет GPUI + Kit на macOS.
- `iroh-automerge` помогает исследовать транспорт и объединение, но не реализует
  нужные Taypeer блокировку, парольный файл, допуск и ротацию.
- KeePassXC — ориентир взаимодействия и средство проверки KDBX. Копирование кода
  требует отдельного учёта его лицензии; поведение продукта определяется spec.md.

UI-ориентиры, исходники и атрибуция иконок находятся в [wireframes](../wireframes/README.md).
Ссылки на остальные upstream и нормативные материалы собраны в spec.md.
Ревизии референсов не следует автоматически переносить в Cargo.toml.

## Инструменты архитектуры

LikeC4 CLI **1.59.3**, pnpm **10.17.1**, проектный Node.js **22.22.3** закреплены
в package.json и pnpm-lock.yaml. Для разбора Markdown используются markdown-it
14.1.0 и github-slugger 2.0.0. Это инструменты документации, не Rust-зависимости.

[Официальный likec4-dsl](https://github.com/likec4/likec4/tree/43f0b705d668a56055a14755f7564c36d1bb2e5a/skills/likec4-dsl)
установлен в `.agents/skills/likec4-dsl` из выпуска `v1.59.3`,
ревизия `43f0b705d668a56055a14755f7564c36d1bb2e5a`, лицензия MIT.
При обновлении сохранять происхождение и проверять примеры установленным CLI:
справочник не заменяет парсер закреплённого выпуска.
[Локальный навык процесса](../.agents/skills/taypeer-architecture/SKILL.md)
описывает только правила Taypeer.
