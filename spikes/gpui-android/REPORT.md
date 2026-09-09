# Результат проверки GPUI + GPUI Kit

Дата: 2026-09-08. **Гипотеза общего готового стека для macOS и Android не подтверждена.**
Эксперимент выполнен частично: настоящий Kit собран и запущен на Mac;
проверенный ранний Kit не компилируется с мобильным GPUI. Android APK и
матрица реального устройства не выполнены. Это не доказательство невозможности порта.

Дополнительно изучен [gpui-mobile](GPUI-MOBILE-REVIEW.md) на закреплённой ревизии:
найдены Android host и отдельные платформенные интеграции, но готовая совместимость
с Kit не подтверждена; остаются расхождения API, заглушки Platform и пробелы IME.
Это обзор исходников, а не новая сборка или пройденная матрица.

## Что проверено

| Кандидат | Фактический результат |
| --- | --- |
| Kit 0.6.0 + GPUI-pre 0.3.4, Rust 1.93.1 | Ошибка `std::hint::cold_path` в GPUI-pre; текущего toolchain каркаса недостаточно для этого lockfile |
| Тот же Kit, установленный Rust 1.97.1 | `cargo check` и `cargo build --locked` успешны; `.app` реально запущен на Mac |
| Kit 0.5.2 + GPUI из Zedra | Cargo разрешил один мобильный GPUI и дошёл до компонентов: **79 ошибок API**; отдельно отсутствует утилита `metal` |
| Android | Исходники проверены; SDK/NDK, рабочий JDK, adb и Rust Android target отсутствуют. APK не собирался, доступность телефона через adb определить нельзя |

Последний Kit использует семейство packages `gpui-pre`, а не `gpui` мобильного
форка. В `gpui-pre-platform` 0.3.4 функция `current_platform` имеет реализации
macOS, Windows, Linux/FreeBSD и Web, но не Android. Простое добавление
`gpui_android` другой ветки не обеспечивает совместимости типов `App`, `Window`
и компонентов. Нужен согласованный порт backend/API либо более старый совместимый
набор, который ещё предстоит найти.

Проверен и более ранний вариант: Kit 0.5.2 до перехода на gpui-pre. В его
workspace пять зависимостей (`gpui`, `gpui_platform`, `gpui_web`, `gpui_macros`,
`reqwest_client`) перепривязаны к одному checkout мобильного Zed. API не менялись.
Компилятор обнаружил отсутствующие `Role`, `AccessibleAction`, `Toggled`,
`OngoingScroll`, `container_query`, `BoxShadow.inset`, методы `role`, `flex_grow_1`
и другие различия. Это фактическая несовместимость **этой пары ревизий**,
независимая от отсутствующего Android SDK. Другие версии не перебирались.

## Зафиксированные версии и окружение

| Объект | Версия / полный SHA |
| --- | --- |
| [Kit, современный](https://github.com/longbridge/gpui-kit/tree/21f6ce6097c686e98e0ca62ff6178bb4612541a1) | `21f6ce6097c686e98e0ca62ff6178bb4612541a1`, package 0.6.0 |
| GPUI современного probe | registry `gpui-pre` и platform/macOS 0.3.4; checksums в `Cargo.lock` |
| [Kit, ранний](https://github.com/longbridge/gpui-kit/tree/e12e39ca55051d02f0e2ae8a910a674905c722d6) | `e12e39ca55051d02f0e2ae8a910a674905c722d6`, package 0.5.2 |
| GPUI в исходном lockfile раннего Kit | `cc053a4a6fa2fd0e8793201ed9099466af1be0b1` (upstream Zed; не мобильный кандидат) |
| [Zedra reference](https://github.com/tanlethanh/zedra/tree/35f86b4b4b33c91365e99bde399374d6b64a3bc2) | `35f86b4b4b33c91365e99bde399374d6b64a3bc2` |
| [Мобильный Zed](https://github.com/tanlethanh/zed/tree/5955915c572ea9336d9648a2d286b399298fe5ff) | `5955915c572ea9336d9648a2d286b399298fe5ff`, gitlink `vendor/zed` Zedra |
| Mac | Apple M4, 10 GPU cores, Metal supported; macOS 26.6.2 (25G83), aarch64 |
| Успешный Rust | `rustc 1.97.1 (8bab26f4f 2026-07-14)`; закреплён локально в spike |
| Неуспешный Rust | `rustc 1.93.1 (01f6ddf75 2026-02-11)` |
| Toolchain мобильного upstream | `1.95.0` в его rust-toolchain.toml; не устанавливался |
| Apple tools | `/Library/Developer/CommandLineTools`; `xcrun metal` недоступен |
| Android reference build | AGP 8.13.2, Kotlin 2.2.10, Gradle 8.14.3, compile/target SDK 36, min SDK 23 |
| Android локально | JDK не найден (`java -version` — Unable to locate a Java Runtime); SDK/NDK/adb/sdkmanager не найдены; NDK версия в reference не закреплена |

macOS 15 не проверялась отдельно. ARM64 Android 12+ и его GPU не проверялись.
Указанный minSdk 23 принадлежит reference, а не меняет требование Pass2P Android 12+.

## Обязательные пробелы Android в выбранном форке

1. [Файловые диалоги](https://github.com/tanlethanh/zed/blob/5955915c572ea9336d9648a2d286b399298fe5ff/crates/gpui_android/src/android/platform.rs#L842):
   `prompt_for_paths` и `prompt_for_new_path` сразу возвращают `Ok(None)`.
   Нужен отдельный SAF/JNI адаптер импорта/экспорта с внутренней рабочей копией.
2. [Буфер](https://github.com/tanlethanh/zed/blob/5955915c572ea9336d9648a2d286b399298fe5ff/crates/gpui_android/android/src/main/kotlin/dev/zed/gpui/GpuiRuntimeController.kt#L148):
   запись использует `ClipData.newPlainText` без чувствительной отметки.
   Пустое значение при недоступности context/clipboard не отделяется от пустого
   буфера. Нужны чувствительная отметка, явная недоступность, владение скопированным
   значением и проверка перед очисткой. [Требование Android](https://developer.android.com/develop/ui/views/touch-and-input/copy-paste#sensitive-content).
3. [IME](https://github.com/tanlethanh/zed/blob/5955915c572ea9336d9648a2d286b399298fe5ff/crates/gpui_android/android/src/main/kotlin/dev/zed/gpui/GpuiSurfaceView.kt#L176):
   тип всегда `TYPE_CLASS_TEXT`; password variation и
   `IME_FLAG_NO_PERSONALIZED_LEARNING` не передаются. Маска Kit сама по себе
   не задаёт безопасные настройки Android IME. [Документация флага](https://developer.android.com/reference/android/view/inputmethod/EditorInfo#IME_FLAG_NO_PERSONALIZED_LEARNING).
4. [Запуск](https://github.com/tanlethanh/zed/blob/5955915c572ea9336d9648a2d286b399298fe5ff/crates/gpui_platform/src/gpui_platform.rs#L50):
   `gpui_platform::application()` на Android паникует с требованием инициализации
   через JNI JavaVM/Activity. Нужен Android host, а не перенос `fn main()`.
5. Android lifecycle hooks присутствуют в Zedra, но Pass2P-удаление ключей,
   сохранение черновика и немедленная блокировка не реализованы и не проверены.
   Биометрический adapter Pass2P также не реализован. Reference не закрывает эти задачи.

Эти адаптеры не заменяют GPUI нативным UI: рендер и компоненты должны остаться GPUI + Kit.

## Фактическая настольная проверка

В `src/main.rs` настоящий Kit Input с маской и reveal, Button, список из 100
искусственных строк с overflow-scroll, переключение en/ru и диалог. Данные не
сохраняются. Значение поля не выводится в label или журнал; отдельно отображается
только число символов. Это технический стенд, не продуктовый интерфейс.

Приложение собрано в `.app` и запущено через computer-use. Первый рендер списка,
поля и кнопок подтверждён. Переключение на русский видно в AX и на снимке после
изменения размера. Маскирование и reveal выполнялись на искусственном тексте.
Однако автоматизированный ввод строки `SYNTHETIC-ONLY-Жук-42` при повторном запуске
дал AX-значение `C-L-Жук-42`; снимки иногда отражали новое состояние только после
resize. Причина (ввод GPUI, автоматизация либо захват кадров) не локализована.
**Корректность текстового ввода и плавность обновления не подтверждены.**

Первый вариант стенда показал диалог, но дублировал его слой поверх Root.
Дублирование удалено, финальный вариант пересобран и запущен; полноценный повтор
проверки диалога не завершён. Снимок [macos-rendered.png](evidence/macos-rendered.png)
относится к финальному варианту и показывает русский UI с masked полем.
[AX ввода](evidence/macos-input-ax.txt) фиксирует несовпадение, а не успешный round trip.

| Сценарий | macOS | Реальный Android |
| --- | --- | --- |
| Сборка настоящего Kit | Пройдена современным кандидатом | Не выполнена |
| Запуск и первый рендер | Подтверждены | Не выполнены |
| Список/форма, фокус, touch | Стенд есть; полная проверка не пройдена | Не выполнены |
| Маска/reveal, en/ru, корректность ввода | Частично; несовпадение ввода требует расследования | Не выполнены |
| Диалог и resize | Частично; финальная проверка диалога открыта | Не выполнены |
| Клавиатура не перекрывает сохранение | Не выполнено | Не выполнено |
| Clipboard, файлы, биометрия | Не реализованы в стенде | Source gaps найдены, runtime не выполнен |
| Фон, lock ОС, уничтожение процесса | Не выполнено | Не выполнено |
| Темы и масштаб 14/16/18 | Не выполнено | Не выполнено |

## Воспроизведение

Команды из корня проекта; в другой среде можно убрать `rtk`.

```sh
rtk cargo +1.97.1 build --locked --manifest-path spikes/gpui-android/Cargo.toml
rtk cargo +1.97.1 run --locked --manifest-path spikes/gpui-android/Cargo.toml
rtk proxy python3 spikes/gpui-android/prepare-legacy.py
rtk cargo +1.97.1 check --locked --manifest-path spikes/gpui-android/legacy/Cargo.toml
```

Явный `+1.97.1` нужен при запуске из корня, где основной проект выбирает 1.93.
Внутри `spikes/gpui-android` toolchain закреплён отдельно.
Команды, реально давшие успешную сборку: `cargo +stable check` и
`cargo +stable build --locked` (stable тогда 1.97.1).

`prepare-legacy.py` получает точные ревизии в ignored `vendor/` и меняет только
пути пяти workspace-зависимостей раннего Kit; не исправляет его API. Отдельный
`legacy/Cargo.lock` фиксирует разрешённые зависимости неуспешного кандидата.
Ожидаемый результат legacy — ошибка, сохранённая в [legacy-errors.txt](evidence/legacy-errors.txt).
Современная сборка: [latest-build.txt](evidence/latest-build.txt).
Ошибка старого Rust: [rust193-error.txt](evidence/rust193-error.txt).
Vendor и target не входят в основной workspace или tracked результаты; `ref/` не изменён.

## Что нужно для закрытия гипотезы

Выбрать и проверить общий GPUI API: порт актуального backend либо совместимый
ранний Kit. Затем реализовать Android host и указанные адаптеры, закрепить
JDK/SDK/NDK, собрать APK и пройти матрицу на настоящем ARM64 Android 12+.
На Mac отдельно разобраться с вводом/обновлением кадров и пройти оставшиеся
сценарии. До этого переносить UI зависимости в основной workspace преждевременно.

## Лицензии и источники

Kit и используемые GPUI crates — Apache-2.0; Zedra — MIT. Это не полный аудит
лицензий транзитивного lockfile. Desktop стенд адаптирован из официальных
[examples/input](https://github.com/longbridge/gpui-kit/blob/21f6ce6097c686e98e0ca62ff6178bb4612541a1/examples/input/src/main.rs)
и API примеров Kit; добавлены сценарии spike и удалён вывод значения в label.
Лицензия upstream приложена в [UPSTREAM-LICENSE-APACHE-2.0.txt](UPSTREAM-LICENSE-APACHE-2.0.txt).
[Официальная установка](https://gpui-kit.com/docs/installation/) перечисляет
настольные ОС, рекомендует Kit 0.6 и Rust 1.90+, но успешная сборка именно
зафиксированного lockfile здесь потребовала более нового Rust. Это различие
проверено компилятором, а не принято по странице документации.
