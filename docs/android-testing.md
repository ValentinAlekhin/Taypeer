# Автоматическая проверка Android

`apps/taypeer-android/qa.py` объединяет сборку, явный выбор ARM64 эмуляторов,
поведенческие тесты, сетевой обмен и визуальную матрицу. Правила БД остаются
в общем Rust runtime. Продуктовые тесты используют настоящий isolated worker;
визуальный harness отдельно рендерит переиспользуемые product builders.
Граница QA, настоящего JNI/worker и local TLS fixture показана в `androidTesting`
[архитектурной модели](../architecture/ui-testing.c4).

Используются только публичные искусственные данные по
[правилам fixtures](../tests/fixtures/README.md). AVD должен быть выделен под QA:
APK устанавливается с сохранением тестового профиля. Runner не выбирает
физические устройства и не очищает весь профиль приложения.

## Подготовка и сборка

Закреплённые JDK/SDK/NDK/Rust описаны в
[Android README](../apps/taypeer-android/README.md#сборка).

```sh
rtk proxy python3 -m pip install -r apps/taypeer-android/requirements-qa.txt
rtk proxy python3 apps/taypeer-android/qa.py doctor
rtk proxy python3 apps/taypeer-android/qa.py host
```

`doctor` читает serial, API, ABI, page size, fingerprint и геометрию AVD.
SDK по умолчанию — `apps/taypeer-android/.tools/android-sdk`, AVD — `.tools/avd`.
`ANDROID_HOME`, `ANDROID_AVD_HOME`, `ANDROID_USER_HOME`, `GRADLE_USER_HOME`
позволяют использовать другую установленную среду. Локальный adb socket
должен быть доступен запуску; ограниченный sandbox может блокировать его.
Перезапускать общий adb server для обхода этого ограничения не требуется.

`host` запускает существующий `check.sh`: тесты QA-утилиты, JVM-тесты, lint,
сборку debug APK/bindings, проверку ARM64 ELF/ZIP 16-КиБ выравнивания.
`--skip-build` у последующих команд применим после успешной сборки текущего
кода. SHA-256 приложения и test APK сохраняется с результатом каждого serial.

## Поведение и сеть

Полный набор на уже запущенных ARM64 Android 12+:

```sh
rtk proxy python3 apps/taypeer-android/qa.py test --serial emulator-5554 --serial emulator-5556 --serial emulator-5558 --authorize-public-biometry
```

Либо runner сам запускает и завершает AVD:

```sh
rtk proxy python3 apps/taypeer-android/qa.py test --avd taypeer-api31 --avd taypeer-api36 --avd taypeer-api36-16k --authorize-public-biometry
```

Это локальные имена подготовленных AVD. На новой машине создаются ARM64
`google_apis` images API 31/36 и `google_apis_ps16k` API 36 штатными SDK tools.
Версию/ревизию образа фиксирует metadata запуска. `--avd` использует cold boot,
без окна/аудио/snapshots, ожидает `sys.boot_completed`, завершает только процесс
этого запуска. `--serial` сохраняет уже запущенный эмулятор.
Не запускать две QA-команды на одном serial одновременно.

Для целевой проверки:

```sh
rtk proxy python3 apps/taypeer-android/qa.py test --skip-build --serial emulator-5554 --suite ui
rtk proxy python3 apps/taypeer-android/qa.py test --skip-build --serial emulator-5558 --suite network
rtk proxy python3 apps/taypeer-android/qa.py test --skip-build --serial emulator-5558 --suite platform
rtk proxy python3 apps/taypeer-android/qa.py test --skip-build --serial emulator-5558 --suite camera-denied
rtk proxy python3 apps/taypeer-android/qa.py test --skip-build --serial emulator-5554 --test-class dev.taypeer.DocumentUiTest
```

| Набор | Состав |
| --- | --- |
| `full` | Все `*Test.kt`, кроме visual harness; отказ камеры и enrolled biometry отдельными стадиями |
| `ui` | Document, generator, preferences, relay settings, invitation, camera, navigation, Activity lifecycle, secret focus, password change, history/comparison и incoming sync → UI; отказ камеры отдельной стадией |
| `network` | P2P реплик, received-source purge/late peer и lifecycle exchange |
| `platform` | Binder, descriptors, Keystore, clipboard, import, attachments, worker; enrolled OS prompt находится в `biometry` |
| `camera-denied` | Настоящий permission dialog: отказ камеры сохраняет ручной ввод |
| `biometry` | Настоящие auth-bound CryptoObject, отмена/EOF/dismissal и recreation без повторного authenticate |

`biometry` и `full` требуют явного `--authorize-public-biometry`: runner на
выделенном ARM64 AVD создаёт PUBLIC PIN/finger, записывает durable boot-bound
receipt **до** изменения lockscreen и восстанавливает исходное состояние в
`finally`. Чужая существующая блокировка не заменяется. Для уже созданного
собственного PUBLIC enrollment у одного explicit target передаётся
`--biometry-state <receipt>`; receipt, boot, user и credential проверяются.
Driver читает только fixed PUBLIC marker своего APK и отправляет finger 1.
Пропуск biometric test, неисполненная exact selection, ошибка driver или restore
дают FAIL отдельной стадии, даже если остальные тесты прошли.

```sh
rtk proxy python3 apps/taypeer-android/qa.py test --skip-build --serial emulator-5554 --suite biometry --authorize-public-biometry
```

На API31 `locksettings get-disabled` требует аутентификацию, когда PUBLIC PIN
уже установлен; helper передаёт только проверенный PIN собственной receipt.
Восстановление подтверждает отсутствие credential и `get-disabled=true`.

`QrPermissionScenario` запускается в отдельной instrumentation. CAMERA отзывается
**до** её начала: Android завершает UID при отзыве runtime permission.
После этой стадии camera-тесты явно выдают разрешение. Предыдущее состояние
CAMERA grant восстанавливается в конце; test-only user flags сбрасываются,
чтобы системный диалог оставался воспроизводимым.
На API 31 отсутствует shell-команда `clear-permission-flags`: только при этом
точном ответе runner передаёт camera-сценарию аргумент для platform TestApi.
Тест с временной shell identity очищает `USER_SET`/`USER_FIXED` исключительно
у `CAMERA` своего пакета и текущего пользователя, проверяет сохранение отказа
и освобождает identity в `finally`. Ошибка доступа не становится fallback;
профиль приложения и разрешения остальных приложений не сбрасываются.
Используется тот же [TestApi pattern CTS](https://android.googlesource.com/platform/cts/+/904a320b8437b32df69ee40d5b8ec3c4794ff01e/tests/tests/permission/src/android/permission/cts/PermissionUtils.java).

Тестовый runner `dev.taypeer.TaypeerTestRunner` отключает публичный relay для
обычных тестов. `network`/`full` и offline/restart сценарий P2P автоматически
запускают `public-relay-fixture`: локальный TLS сервер с временным публичным
CA, строгой проверкой цепочки и loopback SAN. Runner передаёт URL и сертификат
debug-only hook и создаёт `adb reverse --no-rebind`; существующие mappings
не перезаписываются. В `finally` удаляется только созданный mapping, relay
получает newline и корректно завершается. Release не включает этот hook.
Публичный relay и пользовательские сетевые настройки этим тестом не меняются.

При проверке уже собранного APK одновременно с работой над следующим исходным
checkpoint runner принимает `--relay-fixture <executable>` и
`--coverage-sources <captured-java-directory>`. Это явные frozen captures:
relay требует bounded adjacent `manifest.json`, свой SHA-256 и точное совпадение
обоих APK hashes; source capture проверяет product APK и SHA-256 каждого файла.
Несовпадение является отказом. Эти опции сохраняют strict TLS и исходные строки
отчёта; они не разрешают использовать новый код со старым APK. Пока идёт такой
прогон и report, producer build/raw classes запрещено обновлять. Report-only
Gradle читает прежние class IDs. Artifact явно сообщает выбранные classes;
исключённый неуспешный сценарий не становится PASS полной suite.
Для расширенных прогонов с тяжёлой подготовкой можно явно задать
`--timeout 5400`; это общий бюджет instrumentation, не ослабление локальных
ожиданий, lifecycle deadlines или критериев отдельных тестов.

`ReceivedSourcesTest` также получает локальный TLS fixture, включая выбор
одного точного метода через `--test-class`. Оно проверяет поздние источники
закрытых generations после очистки, маскирование конкурентных password origins
и извлечение общей проекции. Тест потерянного ответа проходит Back, повторное
открытие слоя, reset и lock/reopen с прежним extraction OperationId.
`pendingSources` закрытого объекта и `receivedSources` непринятого исходного
change различаются по предметному типу; просмотр не является merge или выдачей
допуска, извлечение создаёт новый объект. Наличие этих сценариев в suite не
объявляет их пройденными: подтверждение берётся из JUnit данного APK.

Сетевые сценарии проверяют реальный общий протокол, приём ciphertext при
блокировке, применение после unlock, offline правки и convergence после
перезапуска host. Это не измерение публичного relay, NAT или OEM-ограничений фона.
Точные условия — [P2P acceptance](p2p-acceptance.md).

`DeviceSettingsTest` и Rust preferences tests проверяют загрузку интервала
автоблокировки при создании Host, сохранение через restart, явный clipboard
no-clean режим, отказ повреждённого файла и отсутствие hot application при
неподтверждённой записи. Настройки сериализуются native Host; Kotlin отображает
только подтверждённый applied view. Clipboard-тесты проверяют собственное/чужое
значение, no-clean через Activity recreation и сохранение прежней квитанции/срока
при подтверждённом отказе публикации. У неопределённого отказа OS rollback не
объявляется подтверждённым.

`PasswordChangeUiTest`, `HistoryUiTest` и `SecretFocusUiTest` относятся к UI suite.
История использует реальный worker, сохранённую версию, текущую сторону сравнения
и подтверждаемое восстановление в выбранную группу. Restore сохраняет ID записи
и добавляет версию истории; clone создаёт новый ID. `RestoreRevisionStateTest`
в platform/full проверяет потерянный durable ответ: точный request переживает
lock/reopen, повтор не откатывает последующее перемещение, acknowledgement
удаляет intent до независимо ошибочного открытия проекции.
`LifecycleStateTest` удерживает точные clone/move requests через Back, другую БД,
lock/reopen и окончательно очищенный источник. Late callback не меняет новую
сессию, подтверждённый receipt удаляется до обновления UI, старый move receipt
не откатывает последующее перемещение.
Focus-only сценарии вызывают настоящий window callback и проверяют отзыв
раскрытых значений и ожидающих ответов без `onStop` или блокировки документа;
несохранённый ввод и параметры генератора сохраняются.
Ротация различает `CONFIRMED`, `REJECTED` и `UNCERTAIN`: только типизированные
`InvalidInput`/`InvalidOptions` до commit разрешают исправить запрос. Неопределённый
результат удерживает прежние пароль, revoke и OperationId для точного повтора;
после подтверждения форма закрывается, ошибка projection не отменяет запись.
`RevisionPropertiesTest` создаёт две реальные сохранённые версии с разными tags,
expiry, Lucide и RGBA. `revisionProperties(entry, revision)` проецирует существующий
Rust `Command::Revision` в тот же DTO, что текущие свойства; чтение не изменяет
ciphertext/history, неизвестная версия и отозванная сессия отклоняются.
Отдельная живая UniFFI-ссылка подтверждает native-отказ после lock, подтверждённого
закрытия worker и открытия нового поколения. Уничтоженная manager-owned обёртка
проверяется отдельно: её `IllegalStateException` не считается доказательством
отзыва доступа в Rust.
Маскированные builder PNG
проверяют композицию, а обновление текущей стороны от incoming и отзыв позднего
защищённого ответа подтверждаются отдельными поведенческими сценариями.

## Визуальная матрица и regression baseline

```sh
rtk proxy python3 apps/taypeer-android/qa.py visual --skip-build --serial emulator-5558
rtk proxy python3 apps/taypeer-android/qa.py visual --skip-build --serial emulator-5558 --case ru-light-small --case ru-dark-font200
```

| Профиль | Язык / тема / шрифт | Телефон, dp | PNG контента, px |
| --- | --- | --- | --- |
| `ru-dark` | ru / dark / 100% | 390 × 844 | 390 × 788 |
| `en-light` | en / light / 100% | 390 × 844 | 390 × 788 |
| `ru-light-small` | ru / light / 100% | 360 × 640 | 360 × 584 |
| `en-dark-large` | en / dark / 100% | 430 × 932 | 430 × 876 |
| `ru-dark-landscape` | ru / dark / 100% | 844 × 390 | 844 × 334 |
| `ru-dark-font200` | ru / dark / 200% | 390 × 844 | 390 × 788 |

Статус 32 dp и navigation 24 dp исключены, как отдельные области FIG.
В screenshot один dp соответствует одному px. Размеры явно передаются Compose
и проверяются у каждого PNG: фиксированный viewport не сможет выдать успешную
матрицу разных телефонов. `wm size/density` восстанавливаются при ошибке.
Язык/тема/font scale передаются harness без глобальной смены настроек Android.

После просмотра PNG и сверки с FIG конкретный device/image profile может стать
reviewed baseline. Runner не принимает новый baseline автоматически:

```sh
rtk proxy python3 apps/taypeer-android/qa.py visual --skip-build --serial emulator-5558 --baseline tests/fixtures/android-ui/api36-16k
```

Baseline directory содержит `environment.json`, по каталогу на каждый case,
`capture.json` и PNG. API/ABI/page size/fingerprint и параметры capture должны
совпасть. По умолчанию допускается нулевая разница. `--pixel-tolerance 1`
допускает разницу канала до 1 из 255; `--max-difference 0.001` — до 0,1%
изменённых пикселей. Любая толерантность требует проверки конкретного renderer;
это не порог соответствия FIG. Drift даёт `comparison.json`, `*-diff.png`
и ненулевой exit code. Для нового baseline сначала просматриваются PNG, затем
явно копируются только synthetic PNG и capture/environment manifests.

Источник композиции — [Android QA макетов](../wireframes/ANDROID-QA.md)
и [основной FIG](../wireframes/taypeer.fig). Pixel regression приложения
и визуальная сверка с FIG — разные проверки, поскольку шрифты/rasterizer различаются.

## Доказательства и границы

Каждая команда создаёт новую `artifacts/android/<UTC>/` либо `--output`:
summary, metadata, APK hashes, raw instrumentation log, JUnit XML, host reports,
PNG/capture profiles, relay log и optional pixel diffs.
`adb` может выйти с кодом 0 при упавшем тесте: runner проверяет отдельные статусы
и terminal instrumentation. Нулевой набор, crash, timeout и незавершённый запуск
дают ошибку. При timeout останавливается instrumentation этого запуска.

Проверка утилиты без SDK/эмулятора входит в Android `check.sh`:

```sh
rtk proxy python3 -m unittest discover -s apps/taypeer-android -p test_qa.py
```

`check.sh` и `qa.py` используют одинаковый `ANDROID_USER_HOME` (по умолчанию
`apps/taypeer-android/.tools/android-user`), включая локальный debug signing key.
Для установленного APK с другим debug key сначала явно согласуется тот же
тестовый ключ или отдельно выбирается чистый AVD; runner не удаляет приложение
и не перезаписывает ключ при несовпадении подписи.

Опциональный JaCoCo отчёт показывает пробелы в Kotlin/Java по объединению JVM
и instrumentation сценариев. Сборка и последующий запуск должны использовать
coverage configuration:

```sh
rtk proxy python3 apps/taypeer-android/qa.py test --coverage --serial emulator-5556 --suite full
```

HTML/XML сохраняются в `artifacts/android/<UTC>/coverage/`. Runner забирает только
private `instrumentation.ec` каждого сценария. UniFFI/AIDL, ресурсы и debug probes
исключены из product метрики. Данные instrumentation объединяются только из текущего
запуска; JVM exec соответствует текущей covered сборке. Генерация отчёта при
ошибке — не успешная приёмка; Rust и отдельные worker процессы не измеряются
этим JVM отчётом. Процент не задаёт произвольного порога: конкретные незакрытые
ветви выбираются для следующих тестов.

Configuration использует закреплённый JaCoCo 0.8.13 по
[AGP coverage](https://developer.android.com/studio/test/coverage-report) и
[Gradle JaCoCo](https://docs.gradle.org/current/userguide/jacoco_plugin.html).

В AGP 8.9.2 шаблон `JacocoTask` для generated `R` —
`.+?/R\$?.*?\.class$` — также совпадает с product-классами `Restore…`,
`Received…`, `Relay…` и оставляет их без проб. Тот же шаблон подтверждён в
[официальных Maven-исходниках AGP 8.10.1](https://dl.google.com/dl/android/maven2/com/android/tools/build/gradle/8.10.1/gradle-8.10.1-sources.jar),
поэтому обновление до этой версии проблему не исправляет. Covered `jacocoDebug`
в своём `doLast` инструментирует только исходные product `R*.class`, исключая
точные `R.class`/`R$*.class` и UniFFI, в уже принадлежащий ему каталог.
Используется [JaCoCo Ant InstrumentTask](https://www.jacoco.org/jacoco/trunk/doc/ant.html#instrument)
с тем же закреплённым 0.8.13. Для отчёта остаются
[исходные классы](https://www.jacoco.org/jacoco/trunk/doc/offline.html#report),
сохраняющие совпадение class ID с execution data.

`verify_coverage.py` проверяет наличие проб именно исходного class ID у
`RestoreRevisionState`, `ReceivedSourcesState`, `RelaySettingsState` после
инструментирования и перед отчётом. Он отказывает при устаревших классах и
не требует проб от пустых интерфейсов. Старый `.ec` без этих проб невозможно
исправить объединением: нужен новый covered APK и повтор сценариев. Процент
из такого старого отчёта не считается достоверной метрикой продуктового покрытия.

Обычные UI-регрессии, настройки, файловая запись/перезапуск, worker, сеть и
virtual camera lifecycle проверяются автоматически. Оптическое качество
настоящей камеры, аппаратный Keystore/биометрия, OEM lifecycle/IME/TalkBack,
физический power-cut и память имеют другие условия. Эмулятор не объявляет
их пройденными. Актуальные результаты — в [testing.md](testing.md), открытые
продуктовые условия — в [roadmap](roadmap.md).

Точный состав реализованного Android UI и оставшихся различий с FIG — в
[матрице Android](../apps/taypeer-android/README.md#матрица-реализации).
Биометрия, перенос рабочего файла и двусторонняя передача управления остаются
реализационными задачами. Имя устройства и изображения имеют промежуточные
Android-доказательства; Access и parent selector должны пройти новый APK.
Наличие кода и покрытие соседнего Rust API не заменяют эту проверку.

Штатные команды: [instrumentation](https://developer.android.com/studio/test/command-line),
[headless emulator](https://developer.android.com/studio/run/emulator-commandline).
