# Проверка gpui-mobile как дополнительного кандидата

Дата: 2026-09-08. Выполнено сопоставление исходников с [предыдущим отчётом](REPORT.md).
**Полезные реализации есть, но готовой совместимой связки GPUI + Kit не найдено.**
Этот обзор не является новой проверкой сборки или устройства и не закрывает гипотезу.

## Ревизии и границы проверки

| Объект | Проверенная ревизия |
| --- | --- |
| [itsbalamurali/gpui-mobile](https://github.com/itsbalamurali/gpui-mobile/tree/1d3ec2a1d14a63b74d1f4269340441d4eeada27a) | `1d3ec2a1d14a63b74d1f4269340441d4eeada27a` — HEAD main при чтении |
| Его [GPUI и gpui_wgpu](https://github.com/itsbalamurali/gpui-mobile/blob/1d3ec2a1d14a63b74d1f4269340441d4eeada27a/Cargo.toml) | Zed `5688167d224b5eca54875d49afb8bfd73a07915a` |
| Kit из предыдущего spike | 0.6.0: `21f6ce6097c686e98e0ca62ff6178bb4612541a1`; 0.5.2: `e12e39ca55051d02f0e2ae8a910a674905c722d6` |

Репозиторий прочитан в отдельном checkout в `/tmp`; `ref/`, исходники стендов,
их lockfile и зависимости основного workspace не менялись. Дополнительно прочитаны
исходники API закреплённого Zed. Новый кандидат не компилировался, APK не собирался,
runtime на Android и macOS не проверялся. Отсутствующий в предыдущей проверке
Android toolchain этим обзором не установлен.

## Совместимость с Kit

1. **Kit 0.6.0 не подключается простой заменой backend.** Он использует
   `gpui-pre` 0.3.4, а gpui-mobile реализует `Platform` из другого package —
   git `gpui`. Переименование зависимости не объединяет типы. Потребуется
   согласовать backend, renderer и API с одним семейством GPUI.
2. **Kit 0.5.2 тоже не получает готовое решение.** В закреплённом GPUI
   [структура BoxShadow](https://github.com/zed-industries/zed/blob/5688167d224b5eca54875d49afb8bfd73a07915a/crates/gpui/src/style.rs#L345)
   не имеет `inset`, а [Kit этой ревизии](https://github.com/longbridge/gpui-kit/blob/e12e39ca55051d02f0e2ae8a910a674905c722d6/crates/ui/src/styled.rs#L39)
   инициализирует это поле. Это конкретное сохраняющееся расхождение API,
   найденное по исходникам. Число 79 относится только к прежней сборке с Zedra;
   количество ошибок новой пары не измерялось.
3. В [демо](https://github.com/itsbalamurali/gpui-mobile/blob/1d3ec2a1d14a63b74d1f4269340441d4eeada27a/example/Cargo.toml)
   нет зависимости на Kit: используются собственные Material/Glass-компоненты.
   Их работа не проверяет обязательные компоненты Pass2P.

## Что можно использовать и что остаётся сделать

| Проблема spike | Найденное решение или заготовка | Остаток для Pass2P |
| --- | --- | --- |
| Android host | `NativeActivity`, `android_main`, передача platform в `Application::with_platform`; создание GPUI-окна после появления native surface | Подключить настоящий Kit на согласованном API |
| Lifecycle и surface | Отложенная обработка Init/TerminateWindow, Pause/Resume и восстановление поверхности | Немедленная блокировка БД, удаление открытых секретов, зашифрованный черновик и проверка уничтожения процесса |
| Touch и прокрутка | Преобразование touch в события GPUI, порог начала прокрутки, инерция, safe-area/insets | Проверить список, форму, диалоги и навигацию Kit на телефоне |
| Файловый диалог | Отдельный package `file_selector` и SAF Java helper | Связать с приложением, читать/писать URI, надёжно сохранять внутреннюю рабочую копию |
| Буфер | Отдельный package `clipboard` вызывает Android ClipboardManager | Чувствительная отметка, различение недоступности и пустоты, проверка перед очисткой |
| Биометрия | Отдельная FragmentActivity с BiometricPrompt | Связать аутентификацию с защитой локального ключа через Keystore; обработать отмену и возврат к паролю |

Host и порядок сохранения `Application` показаны в
[example/src/lib.rs](https://github.com/itsbalamurali/gpui-mobile/blob/1d3ec2a1d14a63b74d1f4269340441d4eeada27a/example/src/lib.rs#L80).
Прокрутка, callbacks и рендер — в
[src/android/window.rs](https://github.com/itsbalamurali/gpui-mobile/blob/1d3ec2a1d14a63b74d1f4269340441d4eeada27a/src/android/window.rs#L1380).

### Ввод — главный дополнительный пробел

`AndroidPlatformWindow` сохраняет `PlatformInputHandler` в set/take, но обработчик
клавиш передаёт текст в глобальный `dispatch_text_input` и отдельно посылает
KeyDown/KeyUp. В этом пути нет вызова сохранённого handler для вставки текста
и IME composition. Демо формы читает собственную очередь `PENDING_TEXT`.
Это требует отдельной интеграции с Input Kit, выделением, заменой текста и IME.
Источники: [handler и клавиши](https://github.com/itsbalamurali/gpui-mobile/blob/1d3ec2a1d14a63b74d1f4269340441d4eeada27a/src/android/window.rs#L1228),
[форма демо](https://github.com/itsbalamurali/gpui-mobile/blob/1d3ec2a1d14a63b74d1f4269340441d4eeada27a/example/src/screens/form.rs#L79).

В [JNI вводе](https://github.com/itsbalamurali/gpui-mobile/blob/1d3ec2a1d14a63b74d1f4269340441d4eeada27a/src/android/jni.rs#L1203)
есть полезное исправление вызова клавиатуры с native thread: используется NDK
`show_soft_input`, обходящий прежнюю ошибку UI thread. Однако аргумент типа
клавиатуры игнорируется; password variation и запрет персонализированного обучения
не настраиваются. Там же в обработке клавиш есть trace-лог Unicode-кодов введённых
символов: перед применением к секретным полям его нужно удалить.

Принудительный рендер через `TEXT_INPUT_DIRTY` обслуживает очередь собственного
мобильного ввода. Это не подтверждённое исправление несовпадения текста или
обновления кадров нашего macOS-стенда с Kit.

### Отдельные packages не заменяют заглушки Platform

В [AndroidPlatform](https://github.com/itsbalamurali/gpui-mobile/blob/1d3ec2a1d14a63b74d1f4269340441d4eeada27a/src/android/platform.rs#L64)
буфер остаётся строкой в памяти процесса, credential store — HashMap,
а `prompt_for_paths` / `prompt_for_new_path` возвращают `Ok(None)`.
Включение features для packages само по себе не переподключает эти методы.

[GpuiFilePicker](https://github.com/itsbalamurali/gpui-mobile/blob/1d3ec2a1d14a63b74d1f4269340441d4eeada27a/example/android/gradle/app/src/main/java/dev/gpui/mobile/GpuiFilePicker.java)
использует ACTION_OPEN_DOCUMENT / ACTION_CREATE_DOCUMENT и возвращает URI.
Это не обычный файловый путь и не подтверждение записи. Вызов блокируется на
CountDownLatch до результата: подключение должно учитывать поток вызова и lifecycle.
Java helpers находятся внутри Android-проекта примера; одной Cargo-зависимости
для их наличия в APK недостаточно.

[GpuiClipboard](https://github.com/itsbalamurali/gpui-mobile/blob/1d3ec2a1d14a63b74d1f4269340441d4eeada27a/example/android/gradle/app/src/main/java/dev/gpui/mobile/GpuiClipboard.java)
пишет обычный ClipData без sensitive-метки. Чтение не выделяет отдельный результат
«нет доступа» для требуемого безопасного поведения таймера Pass2P.
[GpuiAuthActivity](https://github.com/itsbalamurali/gpui-mobile/blob/1d3ec2a1d14a63b74d1f4269340441d4eeada27a/example/android/gradle/app/src/main/java/dev/gpui/mobile/GpuiAuthActivity.java)
возвращает статус аутентификации; вызов BiometricPrompt идёт без CryptoObject.
Готового защищённого хранения ключа БД это не предоставляет.

### Системная тема

ConfigChanged читает night mode и вызывает `set_appearance`, но
`Platform::window_appearance()` всегда возвращает Dark; локальное окно начинает
с Light. В ветке Resume режим заново не запрашивается. Нужен единый источник
режима при старте, смене конфигурации и возврате из фона, затем общий слой темы Kit.
Источники: [platform appearance](https://github.com/itsbalamurali/gpui-mobile/blob/1d3ec2a1d14a63b74d1f4269340441d4eeada27a/src/android/platform.rs#L1069),
[цикл событий](https://github.com/itsbalamurali/gpui-mobile/blob/1d3ec2a1d14a63b74d1f4269340441d4eeada27a/src/android/jni.rs#L692).

## Следующий эксперимент

Приоритет — отдельная попытка портировать мобильный backend/renderer на GPUI API
выбранного Kit. Альтернатива для сравнения трудоёмкости — поиск более раннего Kit,
совместимого с GPUI кандидата; проверенный Kit 0.5.2 уже требует изменений.

Первый критерий такой попытки: единый набор GPUI types, сборка настоящих
Kit Input/Button/Dialog и прохождение текста через стандартный input handler.
Далее — ARM64 APK и синтетический ввод en/ru, composition, выделение, маска/reveal,
клавиатура, прокрутка и уход в фон на устройстве. Только затем имеет смысл
расширять стенд файловыми, clipboard и биометрическими интеграциями из этого reference.

Для воспроизводимости нового стенда нужны собственный lockfile и закреплённые
toolchain/JDK/SDK/NDK. В gpui-mobile Cargo.lock не отслеживается; среди зависимостей
есть wgpu с `branch = "v29"`. В [Android Gradle примера](https://github.com/itsbalamurali/gpui-mobile/blob/1d3ec2a1d14a63b74d1f4269340441d4eeada27a/example/android/gradle/app/build.gradle.kts)
minSdk 26, compile/targetSdk 34; требование Pass2P Android 12+ не меняется.
В [.cargo/config.toml](https://github.com/itsbalamurali/gpui-mobile/blob/1d3ec2a1d14a63b74d1f4269340441d4eeada27a/.cargo/config.toml)
есть настройка `RUST_FONTCONFIG_DLOPEN=on` для кросс-компиляции.
Ошибка отсутствующего `xcrun metal` прежнего macOS-кандидата этим обзором не устранена.

Манифест gpui-mobile указывает альтернативы GPL-3.0-or-later, AGPL-3.0-or-later,
Apache-2.0. Полный аудит транзитивных лицензий не выполнялся.
