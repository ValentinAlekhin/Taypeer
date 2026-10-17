# Проверки

Автоматическая ARM64 Android-матрица, UI/network suites, camera denial и
reviewed visual baseline описаны в [Android QA runbook](android-testing.md).
Этот документ хранит методы и датированные результаты; открытые задачи принадлежат
[roadmap](roadmap.md). Результаты старых APK, dev-схем и удалённых клиентов
не переносятся на новый исходный checkpoint автоматически.

## Локальный аудит desktop — 2026-10-17

Архитектура, локальные ссылки, Rustfmt и `cargo check-all` прошли.
Все 23 сценария настоящего AppView прошли отдельной командой из
[UI runbook](#быстрые-rust-сценарии-macos-ui).
Общий `sh scripts/check.sh` остановился на Clippy:
`large_enum_variant` у `Reply::Management` в
[wire.rs](../crates/taypeer-sync/src/wire.rs). Остальные этапы этого запуска
после Clippy не выполнялись. Исторические полные PASS ниже этого отказа не отменяют.

Аудит сопоставил требования с desktop UI и общими командами runtime;
[подтверждённые функциональные пробелы](roadmap.md#desktop-незавершённые-функции)
сохранены отдельно от native/security acceptance.
Нативные окна, Keychain, Linux/Wayland и Android в этом аудите не проверялись.

При последующей чистке документации `check.sh` повторён: архитектура, ссылки,
fmt/check прошли, Clippy остановил его на том же `Reply::Management`.
После сверки статусов LikeC4 полная validation и `pnpm arch:check` прошли;
проверены ссылки всех 28 изменённых Markdown и оба изменённых проектных скилла.
Экспортированы и просмотрены 15 архитектурных views. В PNG `androidProcesses`
есть обрезание правой и нижней частей большого deployment-графа; этот экспорт
не считается полной визуальной проверкой вида. Остальные 14 просмотренных видов
не выявили обрезаний или наложений узлов. Изменения документации не закрывают
открытую платформенную или продуктовую приёмку.

## Android: текущие сценарии и границы приёмки

Runner проверяет настоящий JNI, isolated worker, Binder, Keystore, clipboard,
local direct/TLS exchange, QR virtual camera и UI. `full`/`ui` сначала выполняют
отказ камеры отдельной instrumentation; `network` автоматически подготавливает
строго проверяемый локальный TLS relay для offline/restart и поздних источников.
Отдельные UI classes включают secret focus, password change и history/comparison.

История сравнивает выбранную сохранённую версию и текущую проекцию без записи;
раскрытие/копирование сторон независимо, incoming refresh и lock отзывают старые
защищённые ответы. `ReceivedSourcesTest` различает аутентифицированные поздние
события закрытой generation и непринятые исходные changes. Оно проверяет
маскирование альтернатив, exact origins, новое извлечение и прежний OperationId
при потерянном ответе через Back/Open/reset/lock/reopen. Device settings проверяют
native startup load, durable-before-hot apply, corrupt/invalid rejection,
перезапуск и clipboard no-clean/foreign value/publication denial.
Восстановление версии сохраняет ID записи и создаёт новую Restore-версию;
клонирование отдельно создаёт новый ID. `RestoreRevisionStateTest` проходит
native commit с потерянным ответом, позднее независимое перемещение, смену БД,
lock/reopen и повтор с точным прежним intent. Ошибка открытия проекции после
acknowledgement не превращает надёжную запись в новый restoration request.
`LifecycleStateTest` аналогично удерживает clone/move intent после Back, смены БД,
lock/reopen и очистки источника; поздний callback не возвращает прежний экран.
`SecretFocusUiTest` отдельно вызывает window focus loss без `onStop`, маскирует
защищённый ввод и сохраняет несохранённую форму/параметры генерации. Типизированный
результат ротации отделяет `REJECTED` (`InvalidInput`/`InvalidOptions` до commit)
от `UNCERTAIN`, при котором пароль, revoke и OperationId остаются тем же запросом.
`RevisionPropertiesTest` различает saved/current tags, expiry, icon и RGBA,
проверяет неизменность ciphertext/history при чтении и отказ после lock.
Исторические свойства получаются из общей saved EntryView, без повторной
бизнес-валидации или новой runtime-команды в Android bridge.

Это inventory сценариев, а не результат нового полного прогона. Успех каждого
образа подтверждают его JUnit, environment и APK hashes; общий workspace check,
host build, visual builder matrix и hardware acceptance сообщаются отдельно.
Артефакты новых интегрированных прогонов добавляются после их завершения.
Реализованные действия и открытые gaps перечислены в
[Android README](../apps/taypeer-android/README.md#матрица-реализации), платформенные
условия — в [roadmap](roadmap.md#android-оставшиеся-проверки).

### Последние проверки изображений и нового bridge — 2026-10-07

Промежуточный APK с исправленным завершением выбранного output имеет SHA-256
`c14612abb55f79da8eb85fa5c3f019f6a0e22818d72e7878d3c26d925d98efa8`, test APK —
`f2c8576450ad1d1cc009e5c26e37185f717a9df5e108c4ee657bf14a314cbe14`.
В `icons-completion-bounded-smoke-api31`, `icons-completion-bounded-smoke-api36`
и `icons-completion-bounded-smoke-api36-16k` каждый JUnit подтверждает 17 passed,
0 failed, 0 skipped. Проверены реальные selected descriptors и вложения,
ошибки завершения после fsync/close, точная историческая image-версия и повторный
выбор изображения незаписанной формы через background / повторную аутентификацию.

Прежние `icons-completion-smoke-*` завершились по тайм-ауту при подготовке
70 записей, а не прошли. Подготовка заменена одним настоящим импортом и
69 настоящими clone-командами; проверки видимости, cache eviction и пикселей
сохранены. Повтор `parent-access-eager70-api36-16k` дошёл до всех 70 записей,
затем завершился отказом обновления UI overview за 20 секунд; это FAIL,
причина исследуется. Текущая visual matrix содержит
228 PNG (38 экранов × 6 профилей); она просмотрена, но после исправления размеров
icon sheet и добавления двух форм родителя группы нужна новая матрица.

`parent-access-common-check-linted.log` подтверждает полный общий
`scripts/check.sh`: LikeC4, fmt/check/Clippy, workspace tests, 23 AppView-сценария,
Rustdoc, release workspace и smoke. Отдельно прошли 12 shared parent-регрессий,
9 runtime Access-регрессий, один настоящий процессный сценарий Access и один
Native credential mapping сценарий. Python QA utility: 40/40.
Covered Parent/Access product APK имеет SHA-256
`9ed0801dd9b6d34359617530c039cdd334e3a44a014f323c83969472e65bb07b`,
test APK после исправления порядка реального draw в PUBLIC fixture —
`15fbcadbd5db485c4bf13a595f7c9fa8adc8bdeb8d6ce2b561b7c9f8653ab8ed`.
`parent-access-target-api31-drawn` подтверждает 21 passed, 0 failed, 0 skipped:
4 Parent UI, 3 Native Access, 6 Access UI, 4 CredentialPublication и 4 Comparison.
На API36 `parent-access-fixture-draw-search-api36` подтверждает 2 passed:
accepted draw/lock/late ACK и поиск по всей БД. Product APK при исправлении
draw fixture не менялся. Первый прерванный общий прогон не является PASS;
его unreadable execution data не используется для покрытия. Новый расширенный
прогон `parent-access-regression-api36-covered-except-eager70` завершился:
153 passed, 1 failed, 0 skipped из 154. Единственный отказ — прежнее ожидание
`Credentials` вместо фактического `ProfileInvalid` после намеренного повреждения
protected credential; ожидание исправлено, но новый Android-повтор ещё нужен.
Eager70 и отдельный camera-denial stage в этот явно выбранный набор не входили.
Его валидный отчёт JaCoCo, связанный с frozen original classes/source и APK выше,
содержит 4325/5136 строк (84,21%) и 3182/5326 ветвей (59,74%). Это промежуточное
покрытие Parent/Access, не полная матрица и не покрытие новых Recovery/Handoff/SAF/Bio.
На этом checkpoint архитектурные flows recovery, handoff и биометрии прошли
whole-model validation. Результаты более поздней продуктовой интеграции приведены
ниже; validation модели сама по себе не подтверждает поведение.

Следующий source checkpoint: shared keyed-open 6 passed, private bounded key FD
reader 4 passed, Runtime management handoff 5 passed (2 protected receipt и
3 настоящих Iroh/worker сценария), весь Native bridge с android-test-support
30 passed. Новый Recovery runtime cold-resume набор ранее дал 10 passed.
Отдельный строго hash-bound read-only audit PUBLIC eager70 ciphertext прошёл:
70 записей, 1 группа, реальные originals/history, нулевые writer callbacks и
неизменный encrypted файл. Он не заменяет ещё не прошедший Android eager70.

### Следующий исходный срез — 2026-10-08

Предыдущий covered host checkpoint `recovery-handoff-saf-host-covered-recovery-closure-fixed`
прошёл JVM, lint, build, ELF/ZIP; app SHA-256
`cd3031834945f05cce588498279574f3bddba382e611edd07f80f2cf6a151a96`, test APK
`038b0108ae8a38e7986eca2da5246f96d9cc2c694fe58ff482da3d4b2890abca`.
API36 regression `recovery-handoff-eager-api36-regression-fixed-covered`:
7 passed, 4 failed, 0 skipped из 11. Все четыре `RecoveryTest` и три ранее
падавшие Management проверки прошли. Три `RecoveryUiTest` получили новую БД,
но преждевременно проверили ещё не обновлённый каталог; ожидание exact нового
root/path исправлено в исходнике. Eager70 прошёл настоящий импорт, но истёк срок
первого preview; теперь перед ожиданием строка действительно прокручивается
в viewport. Эти четыре исправленных проверки ещё требуют новой сборки и запуска.
Не засчитывать этот целевой набор как полный или использовать его покрытие
вместо полной матрицы. Предыдущий checkpoint подтверждает 8/8 SAF и 6/6
Platform/Binder на API36; последняя Native recovery сборка дала 40/40 тестов,
новый runtime OS-close regression — 1/1, существующий recovery набор — 10/10.

Следующий covered host `search-biometry-host-covered-integration-fixed` прошёл
54 Python QA-проверки, JVM, lint, build и ELF/ZIP; app SHA-256
`740e0eca0f3136708ba0f42451c2658385c5adc099c8a69f1dc97829915a2b5c`, test APK
`b50ea30a431fa149358a9d8ea9c56df84a5671bbe8fba062b9ba8e94ce252984`.
`search-biometry-api36-regression-covered`: 12 passed, 2 failed из 14 —
ContextActions 4/4, Recovery UI 3/3, private biometric input port 3/3,
Search UI 2/4. Ошибки поиска воспроизвели исчезновение результатов при
обновлении и устаревший Compose selector после прокрутки; исправления в
исходниках пока не подтверждены повторным прогоном. Корректный отдельный
`eager-seventy-api31-covered` дошёл до настоящего импорта и видимой первой
строки, но первый Native preview не уложился в прежние 20 секунд. Проверки
всех 70 строк, eviction/revisit и pixels не достигнуты; результат остаётся FAIL.
Для него добавлен bounded DEBUG-only счётчик фазы, без чтения содержимого БД.

`product-biometry-api36-16k-settings-wait-covered` выполняет реальные действия
MainActivity/Settings и OS CryptoObject: 2 passed, 1 failed из 3. Прошли
включение, сохранённое состояние, biometric unlock, отключение exact generation
и отмена системного диалога с master fallback. Сценарий ухода в фон после
OS success до provider выявил повторную загрузку статуса из фонового Compose
экрана; ограничение загрузки состоянием RESUMED внесено, но ещё не проверено
новой сборкой. Runner дополнительно отмечает провал всей biometry stage;
это не четвёртый продуктовый сценарий. PUBLIC enrollment receipt и stage
подтверждают восстановление исходной блокировки. Report-only JaCoCo этих
трёх targeted запусков завершён на неизменённых classes и frozen Java capture.
Это не полный набор трёх AVD и не доказательство аппаратной биометрии.

Переданная промежуточная сборка `intermediate-2026-10-08/Taypeer-intermediate.apk`
имеет иной SHA-256 `9d181beea2fee2810856577d237c4143b645924ce0bcd92480b3159cb9cd0a99`.
Она не включает новую продуктовую биометрию и поиск по нескольким БД.

Прямой owned FD путь подтверждён отдельным стендом с actual generated Native
library/bindings: `biometric-safe-native-fd-api31` и
`biometric-safe-native-fd-api36-16k` — по 13/13; actual OS-auth отдельные наборы
`biometric-safe-native-os-auth-api31` и
`biometric-safe-native-os-auth-api36-16k` — по 3/3. Проверяется настоящий
isolated UID, synchronous safe dup удерживаемого PFD, exact32+EOF и отказ
file/socket/writer/nonblocking/короткого/лишнего/неверного PUBLIC input.
Rust OAEP → actual Keystore CryptoObject → JNI → isolated Native проходит
на обеих версиях. Product `BiometricPromptCoordinatorTest` — 2/2 на API31
и 2/2 на API36/16 КиБ; UI lifecycle продолжает использовать настоящий OS prompt.
Это отдельное подтверждение FD/OS-auth границы; продуктовый enrollment/unlock/settings
набор имеет собственный результат выше. Исторический standalone API31 early-cancel
FAIL ниже относится к прежнему checkpoint.

Runner имеет 54/54 Python-проверок. Полный набор явно включает biometry stage
и требует `--authorize-public-biometry`; `platform` его не включает. PUBLIC
PIN/finger setup и restore используют same-boot receipt и проверенную пустую
credential, без предположения, что ошибка означает отсутствие блокировки.
Оба созданных runner receipts API31/API36-16K восстановлены (`restored=true`).
Физический sensor/TEE/StrongBox/OEM и очистка всех provider copies не подтверждены.

Ciphertext Save a copy прошёл 11 отдельных Rust/Native проверок: storage 2,
runtime 4, Native 5. Проверены точный encrypted archive, сохранённые history/blobs,
исключение unfinished local sidecar, сохранность захваченного inode после нового
Save/закрытия host, bounded output, отказ finish/write, отмена и отсутствие ACK
для повреждённого или неполного источника. Эти 11 проверок относятся к Rust/Native. Отдельный более поздний SAF-набор 8/8
на API36 указан выше; новая полная матрица трёх образов остаётся открытой.

Проекция списка прошла два causal regression теста; query-local индекс operation
timestamps сохраняет свежие проверки snapshot и отбрасывает декодированные
revisions. Сервисный audit того же corpus дал GroupInfo 1307 мс и Entries 1259 мс
вместо 2057/2007 мс до изменения; это локальная read-only проверка, не Android
deadline. Коммитируемый PUBLIC corpus с точным checksum теперь проверяется
обычным `cargo test -p taypeer-services --test android_public_eager_fixture`.
Eager70 Android-тест открывает его через настоящий Native worker и дополнительно
посещает все 70 строк, проверяя предел 64 previews и eviction/revisit.

Отдельный `biometric-os-rust-auth-api36-16k-platform-cancel-comparison`:
12 passed, 0 failed, 0 skipped. Настоящие per-use Keystore key, новый Cipher,
OS CryptoObject, product JNI, exact32+EOF и Rust OAEP wrapper проверены;
это не managed worker enrollment или аппаратная приёмка. API31 standalone
ранняя отмена AndroidX prompt (`biometric-api31-early-androidx-cancel-independent`)
остаётся FAIL. Отдельная отмена после rejected PUBLIC finger
(`biometric-api31-started-cancel-independent`) прошла 1/1; объединённый набор
показывает нестабильную очистку OS requests и не считается PASS.
Настоящий isolated UID (`biometric-api31-isolated-pipe-diagnostic`) отказал
в `/proc/self/fd` reopen с EACCES, но прочитал тот же granted FIFO напрямую
(exact32 PUBLIC bytes и EOF). Эта первоначальная диагностика потребовала direct safe dup; его последующее
подтверждение — наборы 13/13 и OS-auth 3/3 выше. Отказ старого standalone prompt
не заменяет результат более поздней продуктовой интеграции.
Native JNI библиотека экспортирует только три объявленные JNI-функции;
копии STL symbols скрыты. Hardware security level, OEM и память не подтверждены.

Промежуточные `rust-icons-*-coverage.json` не используются как итоговая метрика:
изменение исходников между LLVM-прогонами смешало карты покрытия. Причина записана
в `artifacts/android/rust-icons-coverage-validity.md`; необходим чистый прогон
после заморозки Rust. Kotlin-метрика следующего абзаца относится только к APK
имени устройства, а не к текущим изображениям, родителю или Access.

Следующая промежуточная сборка с локальным именем устройства имеет SHA-256
`3ba4d52559d3647da827e3b2ef32172b60cbda410219d9e27df56ea383eff967`, test APK —
`12495abdec129c7d245233915e8b53c594bdec696a7214b68f6f70b9977d126e`.
`device-name-host-rerun` подтверждает 28 Python-проверок runner/coverage guard,
JVM-тесты, lint, APK и 16-КиБ ELF/ZIP. Полные API31/API36/API36-16КиБ прогоны
`device-name-full-*` завершились: каждый основной JUnit содержит 117 passed,
2 failed, 0 skipped из 119; отдельный camera-denial — 1 passed на каждом образе.
Новые проверки имени устройства и relay Native state прошли, кроме общего
`PreferencesUiTest.onlyLocalExchangeLabelUsesAcknowledgedNativeNameInsteadOfUnsubmittedDraft`:
fixture пытался повторно установить content через rule уже открытой MainActivity.
Ещё по одному отказу каждого образа приходится на ожидание маршрута после unlock:
`AndroidNavigationTest.invalidNavigationSaveKeepsTheEditorAndPendingDestination`
(API31), `DocumentUiTest.openingAndUnlockingKeepTheDrawerClosedAndGroupCreationTouchable`
(API36) и `AndroidNavigationTest.lockClearsComparisonAndPasswordDraftSynchronouslyBeforePolling`
(API36 / 16 КиБ). Native проверки очистки и сохранённого файла до unlock прошли;
ожидание реального IME и подтверждения касания проверяется следующим повтором.
Сетевые и shutdown-отказы предыдущей сборки не повторились. Это промежуточная
матрица, пользовательские изображения в её APK ещё не входят.

Исправленный JaCoCo report API36 прошёл guard 42 product-классов с `R` в начале
имени и сопоставление оригинальных class IDs. Для этой сборки Kotlin product
coverage составляет 3731 / 4522 строки (82,5%) и 2569 / 4407 ветвей (58,3%):
`artifacts/android/device-name-full-api36-covered/coverage/coverage.xml`.
`RestoreRevisionState`, `ReceivedSourcesState`, `RelaySettingsState` и
`DeviceSettingsState` имеют реальные исполненные пробы. Это не покрытие Rust
или аппаратных условий и не результат следующей сборки с изображениями.
Повтор общего check завершился после успешных Rust unit-наборов, но rustdoc
обнаружил изменённый во время прогона Native source с ещё не собранной зависимостью
`image`; результат не засчитывается как общий check финального исходного состояния.

Промежуточная сборка с исправленными отказами назначения и явным источником
ciphertext staging имеет SHA-256
`98e3b639855806bddafdf66b83d1ad97e5a60712b71800c4766e15ae0d21ef12`.
Все 8 сценариев `P2PTest`, `ReceivedSourcesTest` и `IncomingUiTest` прошли на
API 31 (`staging-network-api31`). На API 36 прошли регрессии отклонения назначения,
повтора восстановления и закрытых источников; на API 36 / 16 КиБ — все 5
`HistoryUiTest` и 4 `AndroidNavigationTest`. Целевые наборы целиком ещё не прошли:
они выявили две ошибки fixtures — перекрывающиеся корневые элементы relay selector
и создание новой формы вместо `resumeForm` в shutdown regression. Исправленные
fixtures и новое имя устройства проверяются следующей сборкой.

`staging-visual-all` содержит 35 экранов во всех шести профилях, 210 PNG и 6
успешных capture-сценариев. Просмотр выявил дублирование действий на вкладке истории
сохранённой версии; его исправление требует новых снимков. Эти PNG ещё не являются
reviewed regression baseline. `integrated-staging-common-check-rerun.log`
подтверждает полный общий check до добавления локального имени устройства.
Новый JaCoCo guard и report прошли; в целевых execution data появились настоящие
пробы `RestoreRevisionState` и `ReceivedSourcesState`. Общий процент этого
ограниченного набора не является метрикой полного покрытия приложения.

Завершённая интеграционная матрица от 2026-10-07 выявила отказы; она не является
приёмкой. Product debug APK во всех трёх прогонах имеет SHA-256
`62e86755d394041172ce031e14f6b22d00e4256e06f78ba0a15d71b5d6bfe298`.
Каждый образ дополнительно прошёл один отдельный camera-denial тест.

| Образ ARM64 | Основной набор | Артефакты в `artifacts/android/` |
| --- | --- | --- |
| API 31, 4 КиБ | 94 passed / 9 failed / 0 skipped | `integrated-camera-compat-api31` |
| API 36, 4 КиБ | 100 passed / 3 failed / 0 skipped | `integrated-durable-api36` |
| API 36, 16 КиБ | 98 passed / 5 failed / 0 skipped | `integrated-durable-api36-16k` |

Общие отказы связаны с типом отклонения удалённого назначения и извлечения
закрытого источника. На API 31 отдельно воспроизведён `StorageIo` при получении
одобренного приглашения (`api31-enrollment-diagnostic`). API 36 / 16 КиБ также
выявил раннее действие тестовой формы группы и ошибку закрытия worker.
Исправления и новые regression-сценарии требуют новой сборки и повторной матрицы;
успех прежнего APK их не подтверждает. `integrated-common-check-serial.log`
подтверждает завершённый общий check на промежуточном состоянии, отдельно от Android.

В отчёте JaCoCo этого неуспешного API 36 прогона есть данные исполнения,
но пропуски реально вызываемых state-классов ещё проверяются. Его проценты
не используются как достоверная итоговая оценка покрытия. Диагностические
32 PNG в `visual-current-diagnostic-ru-dark` сверены с текущим FIG; они выявили
оставшиеся расхождения, а не закрыли визуальную приёмку.

CI Android job устанавливает закреплённые QA-зависимости и выполняет host
unit/lint/build/packaging checks. Его конфигурация не обещает эмуляторную матрицу,
публичный NAT/relay, физическую камеру, биометрию, IME/TalkBack или memory acceptance.

### Поздние целевые Android-проверки — артефакты 2026-10-08

При актуализации документации сверены более поздние сохранённые summary,
environment и JUnit. Дата здесь относится к файловым артефактам; результаты
привязаны к APK ниже и не являются новым прогоном текущих исходников.

| Checkpoint в `artifacts/android/` | Подтверждённый результат |
| --- | --- |
| `remaining-parity-biometry-api36-16k-negative-ready-covered` | API36/16 КиБ: 6/6 PASS; enrollment/unlock/disable, отмена, background после OS success, поздняя старая подготовка и coordinator; stage success/restored=true |
| `remaining-parity-api36-regression-covered` | API36/4 КиБ: весь выбранный набор 29 тестов / 11 FAIL; внутри Search UI 4/4 и Recovery UI 3/3 PASS, ContextActions 4/4 PASS |
| `recovery-handoff-saf-eager-api36-covered` | API36/4 КиБ: весь набор 30 тестов / 11 FAIL; отдельно CiphertextCopy 5/5 и UI 3/3 PASS, включая настоящий picker/background/cancel |
| `eager-seventy-api31-optimized-export-covered` | API31/4 КиБ: 1/1 FAIL, последний этап `pending/WAITING_BINARY_VIEW`; все 70 preview/cache/pixels не подтверждены |

Биометрия и поздний eager70 использовали app SHA-256
`f23f0aebf18d15bb2ab97397598d2e4eef7cd20eed47e6c501c7cc0d96cfd1d8`, test APK
`8b03dbce71774d1d472890f89e17c9f96576ad51700a77687d818f60b9391412`.
Поиск/Recovery — app `b8ea01f198febd7fedfa5d017545f80958e4abc03689a0048ce5480c58b57ce7`,
test `f18732ab38b4a087aa02f92e1a838d2d7cd43eb81b6c9f8fbd3fca4f62dc136d`.
SAF — app `742e45bb9a0a4f0ec8b7b69e21056f5fd4096845614523d8ab3cadd185b4bf37`,
test `d5d0308d128de67387b2fe3e0b4c82d122e0fa224d6220b9bf94761b239e6a92`.

Оба remaining-parity APK связаны с host summaries и корректными capture всех
90 Kotlin-файлов. Для биометрического APK manifest capture имеет SHA-256
`fa974f5dc16729e5ab4f05ba99a5295ddd8f7575cebe8571f67875d4a4e36263`.
Текущие `DocumentState`, `LibraryNavigation`, `LibraryScreen`, `MetadataHistoryScreen`
и `SearchState` отличаются от capture. Эти PASS уточняют историю исправлений,
но не закрывают новую полную матрицу или аппаратную приёмку. Kotlin capture
сам по себе не подтверждает неизменность всех Native-исходников.

## Приёмка ручного сохранения dev7

Эта матрица задаёт обязательные результаты по [спецификации](../spec.md).
Ручное сохранение dev7 реализовано; результаты конкретных прогонов приведены
ниже. Наличие сценария в таблице не означает его полной платформенной приёмки.

| Сценарий | Наблюдаемый результат |
| --- | --- |
| Три реплики, разные порядки доставки и перезапуски | Одинаковые выбранные значения и дерево; числовой счётчик OpId, затем байты ActorId; часы не влияют |
| Защищённая конкурентная ветка; причинно последующее снятие защиты | Защищённый вариант приоритетен и скрыт от поиска; осознанное снятие защиты доступно |
| Два атрибута с одним именем | Оба ID и значения доступны без обязательного разбора |
| Delete/edit, delete/move, поздние потомки и циклы | Удаление выигрывает; потомки удалённого родителя в корзине; истинные сироты в корне; цикл разорван по рангу связи |
| Обязательная группа | Первая запись требует сохранённую группу; создание/clone/move/restore без активного назначения отклоняются |
| Ввод, 500 мс покоя и ввод во время медленной записи | Пауза не меняет БД; Save подтверждает только захваченный снимок; новый ввод остаётся несохранённым |
| Переход/закрытие изменённой формы | Save / Discard / Stay; ошибка Save оставляет форму, Cancel удаляет только выбранный черновик |
| Несколько форм записи/группы/БД, незавершённая дата, блокировка и перезапуск | Один зашифрованный bundle сохраняет все формы и blobs; нужный объект автоматически продолжает форму; runtime скрывает секреты |
| Пустая неизменённая новая форма | Объект и версия истории не созданы |
| Групповая форма, форма БД, повтор и отсутствие изменений | Одна содержательная порция создаёт одну версию; repeat/no-op/merge не создают версий |
| Потеря ответа, CommitUncertain, sidecar cleanup failure | Прежние snapshot/OperationId дают прежнее подтверждение; автоматическая сверка не дублирует историю и не заменяет новый ввод |
| Read-only, нехватка места, сбой публикации/синхронизации | Нет ложного Saved; прежние данные/черновики доступны; выбор Save сохраняет редактор при ошибке БД, даже если локальный черновик записан |
| Create/Replace, относительный экспорт, существующий путь | No-clobber сохраняет AlreadyExists; неопределённая публикация отличима от отказа до замены |
| Внутренняя копия внешнего файла и повтор создания/получения | Исходник неизменен; нет нового допуска или дубликата; каталог опубликован после надёжной записи |
| Очистка альтернатив/истории и последующее сохранение/поздний обмен | Очищенные значения и объекты не возвращаются; нужные blobs всех локальных форм удерживаются |
| Старый ответ/событие поля, зависший I/O и блокировка | БД, поколение, DraftId и экземпляр редактора проверены; доступ и раскрытый UI закрываются независимо от I/O |
| Приём при блокировке, пропущенное событие, недостающая зависимость и карантин | Ciphertext надёжно принят; применение отдельно после разблокировки; допустимые независимые правки продолжаются |
| Старый dev-файл/неизвестный local bundle и dev7 с неверными required features | Исходные байты не мигрированы, не перезаписаны и не заменены defaults; отказ типизирован |

Desktop-сценарии проходят настоящий AppView и runtime с публичными искусственными
данными, временным профилем и явным worker. При записи проверяется повторное
открытие файла; при обмене — другая реплика. AppView-сценарии выполняются
без управления нативными окнами; для нативного macOS UI используется Peekaboo.
Нативные проверки Linux выполняются в целевой Wayland-среде, Android — на ARM64 Android 12+.
AVD, тестовый clipboard и модель lifecycle не заменяют нативную приёмку.
Отдельно проверить отсутствие лишних уведомлений и исключённых команд KeePass,
бэкапов и восстановления резервного снимка. Будущие стабильные миграции остаются
отдельной матрицей; смена dev-форматов без миграции их не подтверждает.

### Проверка ручного сохранения — 2026-10-07

Один запуск общего `scripts/check.sh` остановился на ошибках компиляции новых
retained runtime-тестов. После исправления оставшиеся этапы выполнены отдельно,
без повторного запуска всего скрипта: check всех targets/features и Clippy,
300 workspace-тестов (6 служебных/generator tests ignored), 3 теста приложения,
компонентная матрица en/ru × light/dark × 14/16/18 и ширины карточек 380/520/614,
23 AppView-сценария (20 и затем 3 после исправления ожидания в тесте), Rustdoc с запретом предупреждений, release workspace и headless smoke.

Android unit-тесты, lint, сборка APK и проверка ELF/ZIP 16-КиБ выравнивания
прошли. Восемь затронутых Compose-сценариев подтверждены на ARM64 API 31 / 4 КиБ:
6 прошли сразу, 2 после исправления ожидания async reload и неоднозначного
селектора в тестах. Повторялся только затронутый тестовый код. Это не проверка
физического устройства, API 36/16-КиБ или полного платформенного/P2P набора.

Новый PUBLIC dev7-корпус создан отдельным генератором; прежний dev6 не переписан.
Обязательная группа проверяется при чтении, слиянии и изменяющих командах.
Сценарии подтверждают отсутствие DB commit после паузы/навигации/SAF,
отдельный Save, сохранение нового ввода после snapshot и безопасную отмену
неопределённой записи. Нативная приёмка Keychain, Linux/Wayland, безопасность
памяти остаются отдельными открытыми проверками. KDBX исключён из v1.

Макеты ручного сохранения выпущены в основном FIG: 172 артборда, verify без
переполнений, узких текстовых блоков и ошибок контрактов. Экспортированы все
172 PNG; просмотрены изменённые desktop/Android-формы и состояния.
Изолированные GPUI-компоненты `editor-card` и `group-form` сравнены с этим FIG:
геометрия действий совпадает; у рамки заголовка редактора остаётся разница
24 px по ширине и 6 px по высоте. Доля изменённых пикселей — 4,24% и 1,82%
соответственно, это информационная метрика, а не порог прохождения.

### Подтверждение программного среза — 2026-10-05

Исторический срез использовал dev6, автосохранение и необязательную группу;
эти правила отменены 2026-10-07. Ниже сохранены уникальные результаты заменённого
плана, а не инструкции текущего клиента.

| Проверка | Датированное подтверждение |
| --- | --- |
| Core/document и storage | 77 и 28 тестов; целевой Clippy прошёл |
| Services | 66 тестов и 2 регрессии повтора/восстановления frozen sidecar; Clippy прошёл |
| Runtime и сеть | 44 теста, 3 native ignored; direct/loopback relay 5 и fork-регрессии 2 прошли |
| Desktop | 6 packages, 13 библиотечных тестов и 14 настоящих AppView-сценариев с worker прошли |
| Android UI | 8 Compose-сценариев на API31, включая тогдашнее автосохранение/фокус, lock/reopen, несколько форм и scripted SAF-return |
| Android platform | По 31 сценарию на API36/PS16K; на API36 P2P повторён отдельно после исправления teardown, также повторён на PS16K |
| Android artifacts | build/lint (0 ошибок, 11 предупреждений), 3 ARM64 ELF, ZIP 16 КиБ и release manifest без debug probes проверены |
| Архитектура и FIG | 8 LikeC4-файлов, 69 элементов, 30 views; изменённые виды просмотрены; 170 артбордов опубликованы локально |
| Общая интеграция | `RUST_TEST_THREADS=2 sh scripts/check.sh` прошёл: модель/ссылки, fmt/check/Clippy, workspace tests, Rustdoc, release и smoke |

Android APK имел SHA-256
`7cdb4be30682c1e06c75fcd315c83c7a05fbb7727ba01203b088273c189866c8`.
Вложения проходили quota-before-read, рост источника, отказ замены, encrypted
continuation, retained history и durable export через настоящий worker.
Descriptor-регрессии проверяли ciphertext во время KDF и отказ после rotation
до/во время credential read. Native-регрессии различали отказ до commit,
CommitUncertain после durable commit и освобождение leases.

У занятого worker доступ отзывался сразу; background-вызов завершался менее чем
за 500 мс, запрос forced termination наблюдался через 2000–2020 мс.
Выход процесса подтверждался отдельно; это измерение не гарантирует срок планировщика.

Удалённый впоследствии CLI прошёл automation, PTY, P2P трёх профилей и точный
повтор восстановления доверия с `--public-fixture-profile`. Обычный check
не вызывал Keychain. Заключительные клиентские этапы зафиксированы в
`795882e`, `98d1a92`, `9fe6dca`, `ec12c92`.

Ожидание асинхронных вкладок использовало реальные данные и deadline.
Scripted SAF не подтверждает системный picker; искусственные I/O/allocator-отказы
не заменяют power-cut, физический Android, IME/TalkBack или приёмку памяти.

### Исторический Android-фундамент — 2026-09-22

На ARM64 Google APIs API31 rev11 и API36 rev7 первоначальные 8 instrumentation
дали по 7 PASS / 1 FAIL (`ProfileBusy`); отдельный recreation-сценарий прошёл.
На API36 PS16K rev7 после JNA 5.17.0 набор дал 8 PASS / 1 FAIL:
прежний `JNI_OnLoad` JNA 5.16.0 больше не воспроизводился.

Причину первого host lock подтвердил отдельный std-only PUBLIC file-lock probe:
на API31/API36 `File::try_lock` вернул Unsupported. Android storage перешёл
на rustix flock, runtime разделил Busy и Io. SELinux запретил hard link;
импорт стал резервировать приватный каталог через mkdir и публиковать staging
через rename с fsync обеих директорий.

После исправлений на API31/API36/API36-16КиБ прошли по 10/10, включая isolated
проверку архива через anonymous ciphertext descriptors. Они удерживали PFD,
использовали позиционный CiphertextIo и FFI-порции до 64 КиБ без host-путей
и plaintext temp. Повтор descriptor-сценария на API31/API36 с отказом allocator
и повреждённым архивом сохранил источник. Rust-регрессии проверяли независимые
смещения, жизнь после unlink и сохранность БД/черновика при отказе staging.

Три Rust-теста Android bridge, APK/bindings/lint и ELF/ZIP 16-КиБ прошли.
Общий check на `b9e36a5` с двумя потоками вне sandbox прошёл полностью,
включая 8 AppView-сценариев. Первый sandbox-прогон не разрешил loopback;
favicon-тест при прежней параллельности отдельно падал, затем прошёл.
Native Keychain не запускался. Этот фундамент ещё не подтверждал продуктовый
документный worker; его интеграция и новые результаты описаны выше.

## Общая проверка проекта

`sh scripts/check.sh` запускает `pnpm arch:check`, затем Rustfmt, проверку targets
workspace, Clippy без предупреждений, тестовый harness, Rustdoc, release-сборку
и конечный `--smoke-test` без GUI. Для LikeC4 проверяются форматирование, полная модель и раскладка,
непустые представления, статусы, повторные связи и ссылки на локальные документы
и исходники. Проверяются ссылки активных Markdown-документов и якоря их целей;
внешние URL и архивные отчёты не обходятся рекурсивно. Тесты разбора ссылок
проверяют ссылки-определения, кодовые примеры, кириллицу и отсутствующие цели.

Проверки артефактов не доказывают соответствия поведения кода архитектуре
или порядка работы агента. Поведенческие тесты находятся рядом с ядром,
адаптером документа и сервисом; общий smoke вызывает реальные команды
демонстрационной БД. Они не заменяют UI-приёмку или проверку зашифрованного файла.

CI выполняет общую проверку на macOS и Ubuntu; на Ubuntu дополнительно запускает
`pnpm arch:build` и сохраняет сайт как артефакт. До первого удалённого запуска
это конфигурация CI и локальные результаты. Отдельный
[Android workflow](../.github/workflows/android.yml) устанавливает SDK/NDK и
выполняет host checks; AVD UI/network матрица пока запускается локально runner.
GPUI подключается на macOS и Linux; обе платформы запускают общие Rust UI-сценарии.
Production Nix-пакет отдельно собирается в CI и проверяет приватный worker. Runner `macos-15` соответствует ARM64 согласно
[справочнику GitHub](https://docs.github.com/en/actions/reference/runners/github-hosted-runners).
Сборка GUI в CI не означает его интерактивного запуска или визуальной приёмки.

## Изолированный рендер компонентов

Инструмент для визуальной работы агента находится в
[taypeer-ui-testing](../.agents/skills/taypeer-ui-testing/SKILL.md).
Runner в `taypeer` рендерит общие компоненты и панели владельцев функций через внеэкранный Metal,
без `AppView`, runtime-сессии, worker, БД и видимого окна. Основной результат —
PNG для просмотра агентом и JSON с метриками; в продуктовый UI инструмент не входит.

Из корня репозитория, на машине владельца через RTK:

```sh
python3 .agents/skills/taypeer-ui-testing/scripts/compare-components.py --list
python3 .agents/skills/taypeer-ui-testing/scripts/compare-components.py --case entry-row-title-rest
python3 .agents/skills/taypeer-ui-testing/scripts/compare-components.py
```

Требуются macOS с Metal, OpenPencil CLI **0.14.0** и Python с Pillow **12.1.1**
из [существующих требований](../wireframes/requirements.txt). Установка CLI:
`bun add -g @open-pencil/cli@0.14.0`. Рендер внутри ограниченного sandbox может
потребовать доступа к службам macOS; видимые окна при этом не создаются.
Linux в текущей закреплённой версии GPUI не имеет headless renderer:
workspace-проверка явно сообщает пропуск, запрос PNG завершается ошибкой.

[Каталог](../apps/taypeer/tests/components/cases.json), версия 2,
содержит три поля (`input-row-rest`, `input-standalone-rest`,
`input-standalone-focus`) и полные строки `entry-row-title-rest`,
`entry-row-url-rest`, карточки просмотра/редактирования, дерево, вкладки,
генератор, формы и содержимое обмена. Общие builders `style::entry_field`, `input_control` и
`input_row` используются редактором и стендом; поля строки сохраняют продуктовую
настройку `appearance(false)`. Обработчики действий остаются в редакторе.
Текст публичный, тема и Inter загружаются из продуктовых ресурсов.

Скрипт читает сохранённый `wireframes/taypeer.fig`. Цель задаётся именами страницы,
цепочкой контейнеров и узлом; обязательные геометрические якоря должны быть
однозначными. Другой текст, отсутствующая или неоднозначная цель — ошибка подготовки.
У строк фон, метка, поле, кнопки и разделитель являются соседями в `Editor`:
экспортируется весь контейнер, затем выделяется объявленная область строки.
Исходный экспорт сохраняется в `design.png`, область — в `region.png`.
FIG и продуктовые стили скрипт не меняет. Сравнение FIG — dark/16; остальные
темы и размеры доступны отдельно для рендера.

В `artifacts/component-checks/<case>/` сохраняются `actual.png`, `design.png`,
`reference.png`, `overlay.png`, `diff.png`, `comparison.png` и `layout.png`.
Агент сначала открывает `layout.png`, читает `report.json`, затем смотрит
`comparison.png` для цветов, текста и состояний. После исправления повторяет тот
же случай. `--case` можно повторить; `--output` задаёт каталог. JSON перечисляет
только текущий запуск; старые PNG других случаев не являются его результатом.
Ошибка подготовки не заменяет предыдущий отчёт.

PNG сохраняют родной масштаб **2×**. Общий технический отступ **4 логических px**
удерживает внешнюю обводку; он исключается из координат layout. Прозрачный FIG
компонуется на фоне родителя. Ширина задаёт доступную область. Поля и строки измеряются после layout;
для целой панели каталог задаёт viewport, а содержимое рисует реальный builder. Из неиспользуемого
внеэкранного viewport удаляется только пустое пространство; выход геометрии за
capture viewport даёт явную ошибку. При разных размерах сравнение дополняет
изображения фоном до общего холста, сохраняя координаты и весь захваченный рисунок;
масштабирование и маскирование расхождений запрещены.

Отчёт версии 2 содержит границы компонента, метки, поля, текстовой области и
действий в логических px относительно начала компонента. Из них рассчитываются
внешние отступы, расстояния между меткой/полем/действиями, вертикальное выравнивание
и внутренние отступы поля. Дельта = **GPUI − FIG**. Это расстояния между layout
областями, а не восстановленные объявления CSS padding/margin и не границы
нарисованных букв. Отсутствующие/дополнительные действия сообщаются отдельно.
`layout.png` показывает нумерованные границы и таблицу значений FIG/GPUI/дельта.
JSON также сохраняет хеши, версии и пиксельные метрики. GPUI использует встроенный
Inter 4.1; Inter референса разрешает CLI, совпадение файлов шрифта не подтверждено.

**Pixel perfect не требуется**. Нужно соблюдать композицию, размеры, отступы,
выравнивание, иерархию текста, цветовые роли и состояния макета. Небольшие различия
сглаживания и растеризации допустимы; систематическое смещение элементов — нет.
Нулевая разница RGB не является критерием приёмки.

По умолчанию расхождения геометрии и пикселей — диагностика с exit 0. Для явно
заданного пиксельного критерия можно добавить `--pixel-tolerance 8 --max-changed-percent 1`: превышение даёт exit 1, ошибка подготовки — exit 2.
Каталог включает целые карточки и панели, собранные владельцами функций через
общие с приложением builders. Для содержимого диалога сравнивается именованная
область содержимого FIG; оболочка диалога не имитируется в тесте. Дополнительные
якоря задаются через `observations` и одноимённые роли `design.anchors`.

Только GPUI-рендер, включая другие темы и масштабы:

```sh
cargo test -p taypeer --features component-rendering --test components --locked -- --output artifacts/component-render --case input-row-rest --theme light --font-size 18
cargo test -p taypeer --features component-rendering --test components --locked
```

Без `--output` target проверяет все выбранные случаи × en/ru × light/dark × 14/16/18,
активное тестовое окно, видимость, фокус, семейство Inter, размер и повторяемость PNG.
Карточки и их вкладки дополнительно проходят ширины 380/520/614; `--width`
задаёт одну ширину для отдельного запуска. `harness = false` сохраняет главный поток, необходимый AppKit.
Матрица запускается отдельным target в `scripts/check.sh`; OpenPencil
и Pillow для общей Rust-проверки не требуются. Рендер-компоненты проверяют вид,
а действия и надёжный результат операции по-прежнему проходят настоящие AppView-сценарии.

Проверка метрик, несовпадения размеров и сохранения внешней рамки:
`python3 -m unittest discover -s .agents/skills/taypeer-ui-testing/scripts -p 'test_compare_components.py'`.

### Подгонка desktop UI — 2026-10-06

Итоговый `RUST_TEST_THREADS=2 sh scripts/check.sh` прошёл полностью: архитектура
и ссылки, fmt/check/Clippy, workspace tests, 20 настоящих AppView-сценариев,
492 сочетания компонентной матрицы, Rustdoc, release workspace и smoke.
Дополнительно прошли 11 тестов сравнения и 13 тестов workflow макетов.
Нативные Keychain tests штатно пропущены; Peekaboo не использовался.
Опубликованный FIG проверен: 170 артбордов, без переполнений и ошибок контрактов;
полный экспорт совпадает с проверенным кандидатом. После зависания API OpenPencil
владелец явно разрешил публикацию по полной сохранённой контрольной копии;
обычный guard публикации сохранён. Детали находятся в `wireframes/README.md`.

Текущий каталог содержит 24 случая, включая реальные панели настроек, обмена и генератора.
`input-row-focus` сравнивается с тем же FIG-образцом, что и `input-row-rest`:
по уточнению владельца строка записи сохраняет оформление без обводки даже в фокусе.
Standalone-поля сохраняют отдельную рамку фокуса. Сценарий недавних БД дополнительно
сохраняет безопасные welcome PNG в light/dark: названия имеют основной цвет текста,
пути — приглушённый; обычные команды используют общий `style::default_button`.
После этих уточнений `check.sh` прошёл: 504 сочетания компонентной матрицы,
20 AppView-сценариев, Rustdoc, release и smoke. Финальный Clippy всех targets/features
прошёл с `-D warnings`; семь сравнений с FIG просмотрены, у row-поля в фокусе
сохраняется исходная геометрия и отсутствует рамка. Пакет `target/Taypeer.app`
пересобран и подписан локальным сертификатом. Нативная UI-проверка не выполнялась.
Таблица записей сохранена: композиция FIG подогнана к существующему DataTable.
Для карточек дополнительно просмотрены 380 px, ru и шрифт 18 px.
Нативный ColorPicker сохраняет рабочее поведение, но его кнопка отличается от FIG:
закреплённый GPUI Kit не применяет внешние стили и не предоставляет замену trigger.
Измеренное различие высоты текстового контейнера заголовка (20 вместо 26 px)
не означает смещения букв: положение glyph baseline проверено по PNG.
Peekaboo и нативная интерактивная приёмка в этой задаче не использовались.

### Desktop DnD групп — 2026-10-06

Общий `RUST_TEST_THREADS=2 sh scripts/check.sh` прошёл: архитектура, fmt/check/Clippy,
workspace tests, 504 сочетания компонентной матрицы, 21 AppView-сценарий,
Rustdoc, release и smoke. Отдельный сценарий DnD повторён с внеэкранным Metal:
просмотрены PNG подтверждённого дерева и drag preview. После финального уточнения
проверки уже закрытого worker прошли Clippy всех targets/features и повтор DnD
с PNG. Нативная интерактивная проверка DnD
и Keychain не выполнялись.

## Первый демонстрационный срез

Сервисный срез проверяется без окна через `--smoke-test` с feature `ui-test-support`; [запуск](demo.md).
Проверяются предметные ошибки без изменения состояния; независимые поля двух
реплик, конфликт одного поля, очистка против нового значения, сохранение старого
черновика и повторная доставка без лишней истории. Отдельно проверяются поколения
сессий, закрытый доступ, изоляция БД, отсутствие секретов в списках и диагностике.

Интерактивная macOS-проверка выполняется через Peekaboo на публичных образцах:
создание БД → группа → запись → правка → история → блокировка → новый вход.
Дополнительно проверяются несохранённая форма, восстановление тестового черновика,
поиск, темы, en/ru, фокус, прокрутка и минимальное окно. В отчёте указываются
ОС/архитектура, наблюдаемый результат и оставшиеся проверки; AX-значение
не заменяет подтверждение видимого кадра.

После правок представлений выполнить `pnpm arch:dev`, просмотреть затронутые виды,
проверить читаемость и навигацию. После изменения MCP проверить stdio initialize,
список инструментов, чтение проекта/элемента/представления и поиск связей.
При обновлении навыка проверить два сценария: изменение взаимодействия требует
модель до кода; внутренний багфикс допускает обоснованное отсутствие влияния.

## Новый macOS UI — 2026-09-15

[Файловый UI](macos-ui.md) проверяется без окон. Тесты
`worker::tests::presentation` запускают настоящий отдельный worker с искусственными
учётными данными: создание с описанием, атомарная форма группы и повтор операции,
маскированный редактор, сохранение неизменённых секретов, точное время, незавершённая
дата без потери ввода, вложение/экспорт, цвет, история и повторное открытие файла.
Отдельный сценарий проверяет, что доступ для чтения не разрешает изменения метаданных.
Тесты UI-состояния проверяют принадлежность результатов запросу, исключение закрытых
БД и очистку выбора/переходов при блокировке. Проверены сохранение локальных настроек,
последних путей и отказ перезаписывать неизвестный формат. Метаданные документа
проверяются на roundtrip, сохранение неизвестных полей и отказ выбирать конфликт.

Окна приложения и Keychain в интеграционной задаче не запускались по указанию
владельца. Следующие сведения относятся к предшествующему UI на образцах и
не подтверждают нативную интеграцию текущей версии:

На Apple Silicon через Peekaboo проверены создание демобазы и группы с кириллицей,
создание записи, добавление атрибута через диалог, переход между вкладками без
потери черновика, подтверждение в памяти, многострочная заметка, Cmd-L и явное
восстановление заметки после нового входа. Проверены Stay/Do not save,
блокирование Cmd-S под модальной формой, клики по ширине ячеек, длинные строки,
смена en/ru и светлой/системной тёмной темы, окно генератора. После перезапуска
оформление восстановилось, созданные демобазы исчезли. После закрытия формы
создания Cmd-O открывает выбор образца; после разблокировки Cmd-N открывает
редактор. При 1100×720 основные поля и команды
доступны; настройки прокручиваются. Размер подтверждён следующим AX-снимком:
Peekaboo resize сначала вернул прежнюю геометрию до обработки события приложением.

Общий `sh scripts/check.sh` прошёл; нативные Keychain-тесты не запускались.
Представления `macosUi` и `uiEdit` экспортированы и просмотрены.
Локальные снимки — `artifacts/macos-ui-qa/` (игнорируются Git).
Это выборочная UI-проверка: полная матрица клавиатуры, VoiceOver, IME,
автоматическая смена темы ОС, macOS 15 и Android остаются открытыми.

## Проверка двух фронтендов

Текущие команды изолированных стендов:

```sh
sh spikes/gpui-android/build.sh
sh spikes/android-compose/build.sh
sh spikes/android-compose/build.sh :app:connectedDebugAndroidTest
```

Последняя команда требует подключённого ARM64 Android или AVD; установка SDK
описана в [Android README](../spikes/android-compose/README.md).
Общий скрипт эти стенды не запускает. Команды, результаты и ограничения находятся
в [Android-отчёте](../spikes/android-compose/REPORT.md) и
[macOS-отчёте](../spikes/gpui-android/MACOS-2026-09-09.md).
[Решение о двух фронтендах](../spikes/README.md) задаёт текущий стек.
Общие сценарии применяются к обоим UI; приёмка требует реальных целевых устройств.

- Проверить синтетический ввод en/ru, IME composition, выделение, замену,
  удаление, маску/reveal, прокрутку формы и доступность подтверждения при клавиатуре.
- Проверить границу UniFFI: типизированные ошибки, фоновое выполнение тяжёлых
  вызовов, время жизни объектов, отмену и явную блокировку сессии.
- Задержать ответ Rust до блокировки и убедиться, что он не возвращает секрет
  на экран; повторить после нового входа, смены экрана и пересоздания Activity.
- Проверить очистку UI и отказ Rust в доступе после ухода в фон, блокировки ОС
  и ручной блокировки. Saved state, журналы и восстановление после уничтожения
  процесса не должны содержать открытых секретов. JVM/FFI-копии исследуются
  отдельно: очистка видимого поля не доказывает физическое затирание памяти.
- Проверить SAF с внутренней рабочей копией, sensitive clipboard с проверкой
  владения и недоступности, биометрию с защитой ключа средствами ОС и отказом.
- Собрать APK с закреплёнными Cargo/Gradle-зависимостями, Rust/JDK/SDK/NDK,
  проверить соответствие UniFFI-привязок библиотеке и запуск ARM64-библиотеки.

Тесты границы с искусственным состоянием не подтверждают криптографию, надёжность
формата или очистку памяти реальной БД. Полная матрица ниже остаётся обязательной.

## Подписанный обмен основного workspace

Общий runtime проверяет получение ciphertext при блокировке, применение после
unlock, перезапуски профилей и подписанное управление. Тесты Iroh используют
локальные direct/relay-транспорты. Удалённый CLI не является тестовым harness.
Исторические результаты Keychain и TLS relay сохранены в
[отчёте](p2p-acceptance.md), не подтверждая нативную приёмку текущего UI.

## Изолированный прототип обмена

```sh
cargo test --manifest-path spikes/encrypted-sync/Cargo.toml --features network --locked --release
```

Состав сценариев, cross-build/JNI, отрицательная проба памяти, fuzzing и advisory
описаны в [README стенда](../spikes/encrypted-sync/README.md),
результаты и ограничения — в [отчёте](../spikes/encrypted-sync/REPORT.md).
Основной скрипт этот изолированный workspace не запускает. Его успех не подтверждает
безопасность полного формата, Android или продуктовый Iroh.

## Бинарный Rust/runtime-срез

Дополнительные воспроизводимые проверки на публичных синтетических данных:

```sh
cargo test -p taypeer-document -p taypeer-storage binary_tests
cargo test -p taypeer-services
cargo test --locked -p taypeer-runtime worker::tests
```

Тесты создают временные файлы и HTTP-серверы loopback; публичные сайты и
пользовательские учётные данные не используются.

Document проверяет независимость rename/replace, конфликт удаления, историю,
новые ID клона, восстановление ID версии, атомарный receipt и ссылки позднего
источника. Storage проверяет каталог, дедупликацию с проверкой байтов, конечную
аутентификацию, подстановку черновика и поток больше 64 МиБ с буферами до 1 МиБ.
Services проверяет повторное открытие, экспорт прежней версии, отдельный бинарный
черновик и очистку рабочего файла; квоты — точный предел 10 МиБ и объединённые
110 МиБ при сохранении чтения/экспорта/удаления. Бинарные bytes не печатаются
в обычных представлениях и проверяемых сообщениях ошибок.

Локальный HTTP проверяет favicon и относительный `base`, игнорирование ссылок
в скрипте, отсутствие credential-заголовков, ограниченный redirect loop,
отказ слишком большому ответу, внешним SVG-ресурсам и крупному изображению.
Групповая команда проверяется на прямых/дочерних записях, пропуске существующей
иконки и независимой ошибке. IP-проверки запрещают переход публичного запроса
к локальным адресам, включая альтернативные записи loopback и IPv4-mapped IPv6.
Процессные runtime-тесты запускают новый worker; повторы не открывают
исчезнувший исходный файл, не создают историю и не повторяют сетевой запрос.

Эти сценарии не заменяют реальный DNS-rebinding/TLS-стенд, power-cut,
длительное fuzzing, проверку всей сложности декодирования, signed manifest,
приём сетевых пакетов и платформенную приёмку секретов. Матрица ниже
продолжает задавать полные условия, даже когда отдельный локальный тест уже есть.

## Матрица будущей приёмки

### Автоматические проверки режимов совместимости текущего dev-формата

Текущий срез проверяется общим скриптом без нативных учётных данных:

- Core проверяет независимость чтения, записи и получения, неизвестную схему,
  требования возможностей и валидирующую десериализацию дескриптора.
- Trust проверяет подпись требований, запрет их изменения административным
  переходом, соответствие схемы исходному control и отдельную ошибку неизвестной
  версии control, manifest, envelope и source proof.
- Document проверяет отказ неизвестному reader, чтение при неизвестном writer,
  отсутствие дескриптора и сохранение необязательных полей после правки и открытия.
- Managed-сервис проверяет read-only при совместимом чтении, запрет изменений,
  применения, очистки, управления и восстановления доверия. База и отложенный
  зашифрованный черновик сохраняют точные байты; клиент с нужными возможностями
  восстанавливает форму и применяет полученные ранее данные без повторной доставки.
  Отсутствие допуска проверяется отдельно от несовместимости записи.
- Storage и Iroh проверяют получение, повторное открытие и пересылку ciphertext
  неизвестной логической схемы через прямое соединение и локальный relay.
  Это не разрешает неизвестные внешние кодировки или обход допуска и durable ACK.
- Отказ старой dev-схеме без перезаписи проверяется в core/storage/runtime.


Все образцы искусственные. Проверки не реализуют миграции, корпус стабильных
версий, приёмку памяти, Android или desktop UI. Нативная платформенная приёмка остаётся отдельной.

### Фиксированный dev7-корпус

[Образцы и генератор](../tests/fixtures/dev7/README.md) содержат только PUBLIC
данные текущей экспериментальной схемы. Тесты работают с временными копиями;
SHA-256 закрепляет входы, генерация запускается отдельно в новую директорию.

- `cargo test --locked -p taypeer-core -p taypeer-document` проверяет текущий
  дескриптор, reader и сохранение неизвестных optional/конфликтных полей.
- `cargo test --locked -p taypeer-services --lib managed::tests::corpus`
  сверяет ID, историю, origins, корзину, источники и экспорт blob после очистки.
- `cargo test --locked -p taypeer-runtime --lib worker::tests::compatibility`
  проверяет read-only, отзыв поколения, выход worker и точные байты БД/черновика.

Эти проверки входят в общий скрипт. Старые dev5/dev6 сохранены как исторические
байты и отдельные входы негативных/низкоуровневых проверок; текущий reader их
не открывает, миграции не обещаются. Stable corpus, настоящий переход схемы,
native credentials и память требуют отдельных доказательств.

### Полная матрица

Каждая строка — план, не статус успешного прохождения. Полные условия остаются
в разделе «Критерии приёмки» [spec.md](../spec.md).

| Область спецификации | Проверка | Уровень |
| --- | --- | --- |
| Создание и редактирование | Нет начальных групп; активная запись требует активную группу; пустая форма не создаёт объект; новый ID клона | Ядро + UI обеих платформ |
| Черновик и блокировка | Несколько форм и автоматическое продолжение; входящие правки; сбой сохранения не отменяет блокировку | Интеграция + жизненный цикл ОС |
| Поиск | Разблокированные БД, источник результата, исключение паролей/защищённых атрибутов | Ядро + UI |
| Конфликты | Разные поля; несколько значений; delete/edit; параллельные перемещения и циклы | Несколько реплик, перестановки доставки |
| История и корзина | Восстановление новой версией, выбор назначения, сохранность конфликтов при очистке | Ядро + интеграция |
| Зашифрованный обмен | Приём и пересылка в lock, применение в unlock, повторы, разрывы, offline | Несколько процессов + fault injection |
| Транспорт | Прямой обмен, relay, отсутствие связи без relay, продолжение после перезапуска | Сетевая интеграция + устройства |
| Допуск | Истечение 5 минут, однократность, отказ, подтверждение, файл+пароль без допуска | Протокольные негативные сценарии |
| Ротация и управление | Новый вход/биометрия, запоздавший отозванный пир, один управляющий, новый набор | Несколько реплик + восстановление |
| Секреты | Отсутствие ключей чтения у фонового обмена, удаление открытого состояния при lock | Проверка архитектуры и памяти |
| Буфер | 30 секунд, свой интервал, отключение, чужое копирование, Android без фокуса | Подменяемые часы + реальные ОС |
| Надёжное сохранение | Нехватка места, отказ доступа, частичная запись, остановка процесса, повреждённый файл | Fault injection на файловом адаптере |
| Схема и совместимость | Все стабильные форматы, чтение/запись/получение, неизвестные поля, миграции и старые офлайн-правки | Корпус версий + несколько реплик + fault injection |
| Android-файлы | Внутренняя копия, повторное открытие, автоматическое продолжение формы, явный экспорт | Реальное устройство + системный picker |
| Вложения | Границы размеров, дедупликация с историей/корзиной, превышение после merge | Ядро + интеграция хранения |
| Генератор | Диапазоны, исключения, пустой алфавит, равномерная выборка, энтропия | Детерминированные unit/property проверки |
| Тема интерфейса | Три режима, смена ОС, возврат из фона, сохранение выбора, обе встроенные палитры | Логика выбора + UI и жизненный цикл обеих ОС |
| Выпуск и UI | Установка, en/ru, масштаб, клавиатура, touch, фокус и ошибки | Целевые macOS, Linux и Android |

Для проверки случайности не использовать хрупкие тесты вида «два пароля обязаны
различаться». Проверять выборку и ограничения с контролируемым источником, а
подключение криптографического источника — отдельно.

## Модель данных и эволюция схемы

Это матрица приёмки [модели данных](data-model.md) и
[контракта совместимости](compatibility.md). Часть сценариев реализована;
документы и таблицы сами по себе не являются результатом их прогона. Существующий spike проверяет ограниченный документ map/скаляров;
его переход контейнера v2 → v3 не покрывает миграции пользовательской схемы.

### Объекты, история и конфликты

| Сценарий | Обязательный результат |
| --- | --- |
| Копирование файла, ротация, миграция, восстановление управления | `DatabaseId` и ID существующих объектов сохраняются; новая линия доверия не переносит старый допуск или локальную идентичность writer |
| Клонирование записи и поддерева | Новые ID объектов/элементов и новая история, корректные внутренние ссылки; содержимое blobs может переиспользоваться |
| Повтор создания, другое создание с занятым ID, ссылка в другую БД | Идентичная операция не дублируется; другой источник не перезаписывает объект, неверная ссылка не применяется |
| Параллельное сохранение логина и пароля | Оба поля объединены, две исходные сохранённые версии, третьей версии от merge нет |
| Параллельное изменение одного поля, в том числе заметок/тегов | Выбран максимальный числовой OpId; альтернативы удержаны; текст и теги не смешиваются по частям |
| Снятие защиты атрибута и параллельное новое значение | Защищённая атомарная ветка приоритетна, значение скрыто от поиска; последующее осознанное снятие защиты разрешено |
| Переименование атрибута и правка значения; совпадение имён | Стабильные ID, независимые правки объединены; одинаковые имена доступны без обязательного разбора |
| Удаление атрибута/вложения и параллельная правка | Не потеряны конфликтующие данные и ссылки на содержимое |
| Входящая правка после открытия черновика | Подтверждение относится к исходным heads и изменённым полям; невидимые правки не перезаписаны и конфликты не разрешены неявно |
| Новый конкурентный вариант поступил после предыдущей проекции | Проекция пересчитана детерминированно; прежние альтернативы доступны в истории без операции разрешения |
| Параллельные перемещения, цикл, отсутствующий родитель | Одинаковое дерево; минимальная по рангу связь поднята в корень, истинные сироты в корне |
| Удаление поддерева и параллельное создание/перемещение дочернего объекта | Удаление выигрывает, поздние потомки остаются в корзине; служебная пользовательская группа не создаётся |
| Автоматическое объединение и восстановление версии | Merge не создаёт версии; восстановление создаёт новую; старые исходники сохранены, повтор не дублируется |
| Неизвестный автор, пустое/отсутствующее значение | Неизвестные сведения не выданы за подлинные; известная точность и различия значений сохранены |

Для нескольких реплик переставлять порядок доставки, повторять доставку и
перезапускать процессы. Сравнивать не только видимые поля, но и исходные хеши,
причинные связи, `RevisionId`, конфликтные варианты и удерживаемые ссылки.
Часы сдвигать в обе стороны: timestamps не должны разрешать конфликт или выдавать
более старое управление за новое. Пользовательские строки проверить на en и ru.

### Очистка и бинарные объекты

| Сценарий | Обязательный результат |
| --- | --- |
| Один blob в нескольких записях, версиях и корзине | Содержимое учитывается один раз; удаление одной ссылки не удаляет нужный объект |
| Тот же blob как иконка и вложение | В лимите вложений учитывается один раз; только иконка подчиняется отдельному ограничению изображения |
| Явная очистка альтернатив и последующее ручное сохранение | Новые снимки содержат выбранное состояние; очищенные альтернативы не возвращаются; живые ссылки удержаны |
| Повтор старого пакета/снимка после очистки | Очищенные ID не возвращаются в доступную историю/корзину |
| Новая поздняя правка окончательно очищенного объекта | Отдельный устойчивый источник для разбора; восстановление только новым явным действием, повтор не дублируется |
| История очищена одновременно с ещё не виденной новой версией | Новая версия не уничтожается границей времени или слишком широкой отметкой очистки |
| Blob нужен очереди, преобразованию или восстановлению | Физическая очистка его удерживает; неизвестные ссылки источника не считаются отсутствующими |
| Локальный черновик использует удаляемое содержимое | Собственная зашифрованная копия позволяет восстановить черновик после очистки БД |
| Последняя нужная ссылка исчезла | После надёжной фиксации в новом рабочем файле нет байтов blob; старый снимок остаётся отдельным файлом |
| Поздняя правка ссылается на уже освобождённый blob | Содержимое доставляется заново или источник ждёт; не появляется фиктивно готовое вложение |
| Очередь получена раньше blobs; прерывание передачи | ACK только сохранённым единицам; применение зависимой части ждёт содержимого, независимые части могут продолжаться |
| Данные поступили между подсчётом ссылок и фиксацией очистки | Повторная проверка/сериализация writer предотвращает удаление новой живой ссылки |
| Ошибка места или записи при очистке | Старый исправный файл остаётся, успех не показан; очередь не удалена ради лимита |

Сравнивать продуктовый размер вложений отдельно от фактического размера файла,
очереди и черновика. Старые отдельные копии не входят в статистику хранилища.
Не требовать полного сокращения Automerge-журнала
или отсутствия старых значений в старых операциях: такой гарантии первой версии нет.

### Атомарное сохранение без автоматических копий

- Повторные сохранения, сетевой приём, отзыв со сменой пароля и изменение KDF
  не создают `.backups` или `.before-*.taypeer`.
- Прежние копии, включая более десяти файлов в `.backups`, остаются побайтно
  неизменными. Обычный файл на месте `.backups` не мешает записи и `storage usage`.
- В Rust/JSON статистике нет `backup_bytes`; рабочий файл и черновик учитываются.
- Ошибка доступа к рабочему файлу сохраняет прежние данные, черновик и возможность
  повторить операцию. Ошибка подготовки защищённого маркера не публикует кандидат;
  ошибка финального маркера после замены возвращает `CommitUncertain`, повторное
  открытие сверяет состояние. Откат на старый файл по-прежнему отклоняется.

Эти сценарии проверяются на синтетических файлах и профилях, без macOS Keychain.
Они не заменяют power-cut и файловые проверки на целевых ОС.

### Совместимость и миграции

| Сценарий | Обязательный результат |
| --- | --- |
| Каждый стабильный файл → актуальное приложение | Чтение с паролем без управляющего и перезаписи; весь путь миграции доступен без промежуточных установок |
| Последовательные шаги и пропуск нескольких выпусков | Совпадающие данные/ID/история/конфликты; цепочка неизменяемых шагов не зависит от версии UI |
| Новый необязательный атрибут схемы → старый writer → новый reader | После изменения знакомого поля сохранены неизвестное значение, конфликтные варианты и исходные подписанные представления |
| Новая обязательная возможность записи, включая удержание blobs | Старый клиент читает при поддержке чтения, но не редактирует, не очищает и не восстанавливает БД |
| Новая обязательная возможность чтения, в том числе защита | Открытое содержимое не показано; файл сохранён; нет fallback к старому значению enum/default |
| Новая схема внутри поддерживаемой оболочки при lock | Сохранение/пересылка без ключа чтения; отдельное состояние невозможности применения |
| Неизвестный контейнер, CRDT-кодирование, протокол, повреждение | Корректная отдельная причина; нет частичного переписывания и ложного успешного ACK |
| Подмена дескриптора или понижение обязательных возможностей | Обычный CRDT-автор не обходит проверенный контракт управления/схемы |
| Несовместимую миграцию запускает неуправляющее или заблокированное устройство | Отказ без изменения общего состояния и исходного файла |
| Переход подготовлен, затем изменились управление или поколение файла | Устаревший кандидат не фиксируется поверх новых данных |
| Сбой подготовки, записи, замены и ответа после commit | До замены исходный рабочий файл сохранён; после замены нет ложного успеха; повтор по устойчивой записи перехода не создаёт дубликатов |
| Получена только часть нового перехода/документа | Не возникает смесь нового номера схемы со старым представлением, разрешённая для записи |
| Управляющее офлайн при поступлении структурно старых правок | Источники переживают перезапуск; работа с текущей схемой продолжается; перенос возобновляется после доступа к управляющему |
| Преобразование старой правки одновременно с новой правкой поля | Сохраняется настоящая причинность и конфликт; позднее время преобразования не делает его победителем |
| Повторная упаковка/доставка источника после ротации | Одна обработка по исходной идентичности; исходная версия не дублируется |
| Частичный перенос, остановка, затем передача управления | Новый управляющий продолжает необработанные части по переданным отметкам и не повторяет завершённые |
| Несколько миграций вместе с ротациями/отзывом | Исторические ключи не обходят допуск; непринятые зависимости отозванного автора требуют разбора |
| Конфликтующие подписанные переходы схемы | Нет CRDT-победителя управления; остановка обмена/управления и доступное корректное локальное чтение |
| Старое приложение → новое локальное хранилище или черновик | Нет сброса неизвестного состояния в defaults; секретный исходник сохранён до успешного преобразования |
| Локальный черновик старой схемы при недоступном управляющем | Секреты не отправляются управляющему; однозначная форма преобразуется локально, неоднозначная остаётся для разбора до сохранения |

Матрица использует реальные поддерживаемые reader/writer прошлых стабильных
выпусков, а не только текущий парсер с подставленным номером. Для будущих
несовместимых возможностей создаются искусственные данные с ожидаемым отказом.
При каждом переходе сверять идентичность БД, доверенную линию, исходные изменения,
сохранённые версии, blobs, отметки очистки, неприменённые источники и receipts.

Fault injection дополняется завершением процесса и восстановлением локальных
маркеров хранения. Fuzzing проверяет парсеры всех поддерживаемых слоёв, цепочки
миграций, повреждённые зависимости и ресурсные ограничения полного документа.
До реализации эти проверки остаются задачами, не заявлением о доказанной безопасности.
Правила корпуса — в [fixtures](../tests/fixtures/README.md).

## Тема интерфейса: тесты и оставшаяся приёмка

В общей проверке уже пройдены [тесты настроек и ресурсов](../apps/taypeer/crates/taypeer-settings-ui/src/preferences.rs):

- Полнота и совпадение цветовых ролей `light` и `dark` встроенного
  [TOML](../resources/theme.toml), допустимые RGB-значения, совпадение ключей
  и непустые строки локализаций en/ru.
- Значения по умолчанию при отсутствии файла и сохранение языка, размера шрифта
  и выбора `system` при повторном чтении настроек.
- Отказ повреждённым, неизвестным или недопустимым настройкам без перезаписи;
  сохранность предыдущего файла при ошибке записи.

Эти тесты проверяют ресурсы и файловые операции настроек. Остаётся полная
матрица событий ОС и видимого поведения интерфейса на целевых устройствах:

- Проверить выбор активной палитры: системный режим при светлой/тёмной ОС,
  ручная фиксация обоих вариантов и возврат к актуальному режиму ОС. Подменой
  платформенного источника проверить недоступный режим: последний известный
  системный, иначе светлый. Вычисленная палитра не заменяет выбор `system`.
- На macOS, Linux и реальном Android проверить запуск в обоих режимах ОС, переключение
  в обе стороны во время работы и после возвращения из фона. В системном режиме
  все окна, редактор и диалоги обновляются без потери ввода, фокуса и навигации;
  ручной режим не следует за ОС. Возврат Android из фона сохраняет блокировку БД
  и применяет нужную палитру к экрану разблокировки.
- Проверить доступ к выбору темы до открытия и разблокировки БД, немедленное
  применение по общим правилам настроек устройства и сохранение после перезапуска.
  Копирование и синхронизация БД не переносят тему; переключение БД её не меняет.
- На обеих встроенных палитрах проверить en/ru, читаемость текста, фокус,
  выделение, hover, disabled и сообщения состояний. Явные цвета записей
  сохраняются, смена темы не меняет данные БД.

Реализованный macOS demo, тесты настроек и статические FIG не заменяют эту
приёмку. Полная UI/OS-матрица, отдельная проверка macOS 15 и реального Android
остаются открытыми; отдельные наблюдения фиксируются с их ограничениями.
Пользовательский TOML, частичные переопределения, общий файл графического редактора
и сброс цветов проверяются на будущем этапе персонализации, вне первой версии.
Для пользовательских палитр не вводятся проверки или предупреждения о контрасте.

## Организация по мере реализации

- Чистые unit-тесты рядом с модулями, интеграционные — в `tests/` соответствующего crate.
- Общие искусственные файлы — в [tests/fixtures](../tests/fixtures/README.md).
- Контролируемые часы, генерация ID и ввод-вывод — на проверяемых границах,
  без ожидания реальных пяти минут в обычных тестах.
- Тесты сети должны явно выбирать режим direct/relay; обычные тесты не зависят
  от публичных relay, пользовательских файлов или содержимого системного буфера.
- На каждой проверке устройства фиксировать модель, ОС, архитектуру, ревизии,
  шаги, ожидаемый и фактический результат. Эмулятор не заменяет целевой Android.

После выбора зависимостей добавить проверку advisory и лицензий с реальной
политикой проекта. Fuzzing парсеров и будущих миграций добавлять вместе
с этими реализациями, используя корпус без настоящих секретов. KDBX вне v1.

## Runtime и граница процесса

[Процессные проверки](../crates/taypeer-runtime/src/worker/tests.rs) используют
реальный dispatch, сервис и временные PUBLIC-файлы. Они проверяют маскирование,
сохранение и повторное открытие, историю, адресные правки, неверный пароль,
локальные формы, блокировку и ошибку записи. Терминальные PTY-проверки удалены.

```sh
cargo test --locked -p taypeer-runtime
```

CRDT и файловые проверки остаются в document, services и storage. Они не
заменяют power-cut, аудит защиты полного rollback или нативную приёмку UI.

## Нативная связка ключей: отдельная приёмка

CLI и его нативные терминальные проверки удалены. Общий check использует
PUBLIC-credentials и не обращается к пользовательской связке ключей.
Продуктовый credential-адаптер сохранён; успешные искусственные тесты
не подтверждают его текущую нативную приёмку. Флаг `--native-keychain`
удалён из общего скрипта вместе с CLI-specific запуском.

## Защита сессий runtime

[Контракт и воспроизведение](sessions.md#проверки) описывают общий таймер,
реальные зависшие процессы, поздние/частичные ответы, границу истечения срока,
EOF родителя, сохранение и отказ зашифрованного черновика, принудительный выход
и несекретные настройки политики. Эти тесты входят в обычный workspace check и не
вызывают Keychain. Два ignored unit helper запускаются из родительских тестов;
это не нативные проверки credentials. Проверки UI, native lifecycle, Android и памяти
по-прежнему отдельные.


## Быстрые Rust-сценарии macOS UI

```sh
rtk cargo test -p taypeer --features ui-test-support --test ui --locked -- --test-threads=1
```

[Сценарии](../apps/taypeer/tests/ui.rs) создают настоящий AppView и проходят
его обработчики через GPUI Kit. Проверяются создание БД/группы/записи,
сохранение последнего пути и показ на приветственном экране после перезапуска,
открытие из списка последних с обязательным запросом пароля;
редактирование нескольких вкладок и повторное открытие; блокировка с раскрытым
значением и непринятым ответом, завершение worker и автоматическое продолжение формы;
приглашение двух независимых приложений, обмен в обе стороны и сохранение ciphertext
при заблокированном получателе с последующим применением.

`group_drag_changes_order_and_parent_durably` проходит настоящие события мыши
дерева: вставку перед/после, перенос внутрь, возвращение на верхний уровень через
свободное место и отдельную нижнюю зону. Проверяются сохранение выбора группы,
запрет переноса в себя/потомка, сохранение последнего ввода перед
переносом, отсутствие изменения дерева при отказе записи и успешный повтор.
Блокировка во время активного drag прекращает жест и убирает preview.
Порядок, вложенность и введённая запись проверяются после повторного открытия
файла. Фильтр повтора: `cargo test -p taypeer --features ui-test-support --test ui --locked -- group_drag`.

[Session](../apps/taypeer/src/desktop/testing.rs) обслуживает GPUI и реальные потоки
по монотонному сроку ожидания. `wait_idle` ждёт подтверждения фоновых команд,
но не заменяет проверку результата. `disabled() == None` в закреплённом Kit
не доказывает доступность: сценарии дополнительно проверяют подтверждения и результат
действия. Ввод проходит через события; stores не являются публичным тестовым API.
Последовательный запуск нужен из-за rust-i18n. Target использует `harness = false`
и выполняет сценарии последовательно на главном потоке: этого требует macOS
при создании настоящего текстового движка и Metal. Поддерживается фильтр имени
сценария. Общий скрипт запускает их отдельной командой на macOS и Linux,
включая CI. Внеэкранный Metal renderer доступен отдельно на macOS.

Каждый тест владеет временными путями профиля, оформления и файла БД. Отдельный
`taypeer-ui-worker` запускается только с feature `ui-test-support`. Хост и worker
явно выбирают файловый адаптер специально созданных публичных credentials; ошибки
Keychain никогда не переключают production на него. Release без feature не содержит
тестового входа и файлового credential-адаптера. Не менять HOME и не подставлять
пользовательские каталоги. Все значения синтетические, пароль —
`PUBLIC-UI-password-42`. Системный буфер и AppKit helper не используются;
свой буфер передаётся в тестовую платформу GPUI. Файловые диалоги получают ответы
через внутренний `FilePicker`: production вызывает GPUI, тест явно задаёт
ответ до клика и проверяет, что обработчик его запросил. Неожиданный диалог
завершает тест ошибкой. Файловые операции после выбора не подменяются. Relay отключены; нужен доступ к прямым локальным сокетам, который
sandbox может запретить. Публичный интернет для сценариев не нужен.

При падении Session записывает `artifacts/ui/<scenario>/session-*.txt`: последний
шаг, ID элементов, видимость, доступные признаки disabled и безопасные признаки
состояния. Значения, подписи элементов, приглашения и защищённые поля не записываются.
macOS CI сохраняет эти файлы при сбое. Не печатать `ElementSnapshot` целиком:
его Debug содержит значения и подписи. Завершение сессии отзывает доступ и ждёт worker;
удаление временного каталога принадлежит тесту.

Сессия использует `HeadlessAppContext` закреплённого GPUI Kit
`21f6ce6097c686e98e0ca62ff6178bb4612541a1` (GPUI `0.3.4`), ProductAssets и настоящий
текстовый движок. До создания окна регистрируются те же встроенные файлы Inter 4.1,
что и в приложении. `zoom_and_edit_preserve_entry_identity` дополнительно проверяет,
что основной шрифт остаётся Inter после смены масштаба, а латиница, кириллица и
цифры в четырёх начертаниях отображаются без подстановки другого семейства.
Для повтора тех же сценариев с внеэкранным Metal и PNG:

```sh
TAYPEER_UI_PNG=1 cargo test -p taypeer --features ui-test-support --test ui --locked -- --test-threads=1
```

В локальной RTK-сессии: `rtk proxy env TAYPEER_UI_PNG=1 cargo test ...` с теми же
аргументами. Требуется Mac с доступным Metal; отсутствие renderer явно завершает
подготовку ошибкой. Видимых окон нет. PNG сохраняется при завершении сессии, включая
размотку после сбоя, рядом с текстовой диагностикой. При открытом редакторе,
форме, приглашении или раскрытых защищённых значениях PNG не сохраняется;
экран разблокировки допускается только при подтверждённо пустом поле пароля.
сообщение объясняет пропуск. Снимок дерева элементов не выдаётся за изображение.
`Session::with_viewport` задаёт поддерживаемый размер от 1100×720;
`capture_checkpoint` сохраняет именованный безопасный этап с теми же запретами,
а отсутствие renderer возвращает явную ошибку. Имена этапов не содержат значений
полей. Сценарии проверяют минимальный/default viewport, защиту редактора, раскрытых
значений, заполненного unlock, приглашения и получения базы.
Основа соответствует [примеру рендеринга GPUI Kit](https://gpui-kit.com/docs/test/).

`TestAppContext` в `gpui-pre 0.3.5` по-прежнему не принимает ресурсы/renderer;
`HeadlessAppContext` не предоставляет ответы файловых диалогов. Внутренний
интерфейс выбора файла снимает эту зависимость без форка GPUI. Системный Keychain,
IME, настоящие уведомления и оконные диалоги остаются отдельной приёмкой через
Peekaboo и целевые устройства.

### Анимация диалога и фокус ввода в GPUI Kit

В закреплённом Kit `21f6ce6` диалог смещается в течение 250 мс реального времени.
`TestWindowExt::click` вычисляет центр элемента, затем перерисовывает окно при
движении указателя и отправляет mouse-down по прежним координатам. Во время
анимации точка может попасть в отступ поля. Рамка Input имеет отдельный focus
handle: наблюдение `focused() == Some(true)` включает фокус рамки, но не доказывает,
что внутренний текстовый редактор принимает клавиатурный ввод. При исследовании
ошибки подготовки группы каждый символ возвращал `dispatch_keystroke == false`,
значение оставалось пустым, а контекст клавиатуры не содержал `Input`.

Сценарии ждут неизменных bounds видимого элемента в течение 50 мс перед обычным
кликом, заполнением поля и фокусировкой перед вставкой приглашения, продолжая
отрисовку и обслуживание событий. Это окно наблюдения стабильности, а не фиксированная пауза на предполагаемую длительность
анимации; изменение геометрии начинает ожидание заново, общий срок ограничен
`Session::wait`. Продвижение виртуальных часов само по себе анимацию не завершает.
После ввода сохраняется проверка значения публичного поля, после сохранения —
проверка результата и повторное открытие файла.

Диагностическая копия воспроизвела прежний сбой с `SEED=22`; контролируемый клик
в верхний отступ воспроизводил его также с задержками между событиями. После
завершения анимации те же действия принимали все символы. Сценарий
`creation_and_editing_survive_reopening` проверяет ввод в диалогах создания БД,
группы и атрибута с последующим чтением сохранённых данных. Для повтора с seed:

```sh
rtk proxy env SEED=22 cargo test -p taypeer --features ui-test-support --test ui --locked -- creation_and_editing_survive_reopening
```

Это исправление синхронизации тестового драйвера. Особенность фокуса при клике
по отступам остаётся в стороннем компоненте; её исправление и регрессионная
проверка принадлежат GPUI Kit. Нативное поведение мыши и IME этим не проверено.

Проверка исправления 2026-09-22: сценарий создания и редактирования с `SEED=22`,
отдельный сценарий обмена и все восемь AppView-сценариев прошли. Полный
`scripts/check.sh` завершился успешно, включая Clippy, тесты workspace, Rustdoc,
release-сборку и smoke. Peekaboo, Keychain и нативные окна не запускались.

### Проверки после выделения desktop features

`apps/taypeer/tests/ui.rs` сохраняет публичный `taypeer::testing::Session`.
Селекторы повторяемых строк используют domain ID. Дополнительный сценарий
`zoom_and_edit_preserve_entry_identity` проверяет неизменность ID после
редактирования, выбор масштаба клавиатурой, сохранение масштаба и фокус поиска.
Это автоматизированная проверка поведения; нативные overlays, системные события,
буфер и визуальная приёмка человеком остаются отдельными.

Регрессия `inspector_reveal_can_be_hidden_and_revoked` проверяет раскрытие пароля
в карточке просмотра, повторное скрытие и отбрасывание ответа при блокировке.
До исправления воспроизводила `double_lease_panic<WorkspaceStore>`: обработчик
ответа пытался прочитать ещё обновляемый store. Результат теперь передаётся после
завершения его обновления с повторной проверкой сессии, выбора и поколения раскрытия.

## Безопасные повторы обычных команд

Целевые проверки: `cargo test -p taypeer-services --test retries`,
`cargo test -p taypeer-runtime worker::tests::retries -- --test-threads=1`,
`cargo test -p taypeer --features ui-test-support --test ui -- save_failure lock_during_save`.
Сервисные тесты сравнивают байты файла после повторов, количества объектов и
версий, первоначальные результаты, позднейшие значения и типизированные отказы.
Процессные сценарии используют настоящий дочерний worker, зашифрованный архив
и адаптер `CipherPersistence`: отказ до commit, неопределённый результат после
commit, сбой удаления sidecar и блокировку перед доставкой ответа.
Маркеры отказов доступны только синтетическому адаптеру тестовой сборки; обычный
worker их не читает. Фиксированный корпус проверяет sidecar без `attempt`.
AppView проходит ошибку сохранения, повтор кнопкой, исправление формы и блокировку
во время подтверждения; результат проверяется повторным открытием файла.
Нативные окна, Peekaboo и Keychain в эти сценарии не входят.

Локальная проверка 2026-09-22: целевые сервисные, документные и CLI-тесты,
шесть процессных сценариев повторов и оба новых AppView-сценария прошли.
Общий `scripts/check.sh` прошёл архитектуру, форматирование, check-all, Clippy
и неоконный набор тестов, но первоначальный полный UI-прогон был нестабильным: в подготовке
группы `fill(field-name)` иногда наблюдает пустое значение после синтетического
ввода. Воспроизводится также в существующих сценариях, до проверки сохранения;
причина и исправление тестового драйвера описаны выше в разделе об анимации
и фокусе. Проверки не отключены и assertions не ослаблены.
Rustdoc со строгими предупреждениями, release-сборка workspace и smoke-проверка
прошли отдельно. Peekaboo, Keychain и нативная приёмка окон не запускались.
