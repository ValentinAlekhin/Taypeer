# Taypeer Android

Продуктовый Android-проект `dev.taypeer`, Android 12+ / ARM64. Каталог, создание,
импорт, разблокировка, документный редактор и история используют общий Rust
runtime. Независимые формы сохраняются автоматически и продолжаются после
повторной разблокировки. Текущая работа и приоритеты — в
[плане автоматизации](../../docs/automation-plan.md).

Точка продолжения: [состояние и следующие шаги](NEXT.md), 2026-10-05.

## Граница UniFFI

`native/` — отдельный crate основного Cargo workspace. Только он зависит от
UniFFI: типизированные DTO, ошибки, callbacks и преобразования. Общие crates
не зависят от Kotlin/Compose/UniFFI. Генератор вызывает общий сервис; проверка
импортируемого файла использует `RuntimeHost::inspect_compatibility`.
`Host` подключает защищённые credentials и остаётся владельцем ciphertext-runtime.
Его создание не разблокирует БД. Мастер-пароль в credential store не передаётся.

`DocumentService` — непубличный isolated service, обслуживающий ровно одно поколение
открытой БД. Общий supervisor получает независимый локальный Binder death observer
и очередь принудительного завершения, отдельно от занятых командных pipe.
Snapshot, commit и encrypted local forms передаются ограниченными descriptor
leases. Авторская capability выдаётся после аутентификации точного immutable
snapshot с проверкой текущих полномочий до и после чтения credentials.
Desktop spool paths и `open_local` не используются в качестве обходного пути.
Смерть host завершает isolated процесс через Binder death recipient.

UniFFI bridge version 2 экспортирует предметные DTO и команды, включая идентичность
БД, поколения, формы, её ревизии и операции. Приватные JSON-кадры остаются деталью
runtime, универсального публичного API документа нет. Выбранные пользователем
вложения имеют отдельный потоковый Binder-порт, не смешанный с ciphertext I/O.
Каждая порция ограничена 64 КиБ; plaintext временные файлы не создаются.

`KeystoreCredentials` хранит только AES-GCM ciphertext в `noBackupFilesDir`,
с service/account в associated data. Ключ остаётся в Android Keystore;
неподтверждённая запись и потерянный ключ — ошибки, без plaintext fallback.
Физическое затирание копий JVM/FFI этим не доказывается.

Импорт читает SAF только на вход, проверяет внутренний staging-файл общим Rust
reader, затем публикует новую рабочую копию без перезаписи существующей. Частичный
поток и неверный формат не попадают в каталог. Каталог публикуется после успешной
аутентификации worker; импорт не даёт сетевого допуска и сохраняет источник.
Создание и P2P-получение используют внутренние рабочие копии без выбора директории.

Получение ciphertext отделено от применения после разблокировки. Длительный
обмен включается явно через foreground service с тихим обязательным уведомлением;
короткие периоды используют JobScheduler и общую Rust-логику обмена. Kotlin не
создаёт второй протокол допуска или синхронизации. Уход приложения в фон немедленно
отзывает доступ и очищает UI, даже если выполняется команда. Возвращение из SAF
требует новой разблокировки; выбранное вложение привязывается к той же БД и точной
форме новым явным действием, а не запоздавшим callback прежней сессии.

Буфер хранит несекретную квитанцию (случайная метка, HMAC и сроки). HMAC-ключ
неэкспортируемый в Keystore; простой хеш пароля не сохраняется. Проверка владения
повторяется после получения фокуса; чужое значение не удаляется. Очистка UI и
флаг `FLAG_SECURE` не заменяют проверку памяти.

## Сборка

Требуются Rust из корневого `rust-toolchain.toml`, target `aarch64-linux-android`,
JDK 17, SDK platform 35, build-tools 35.0.0, NDK 27.2.12479018.
Версии Gradle/AGP/Kotlin/UniFFI взяты из принятого Compose-spike. Wrapper проверяет
SHA-256 дистрибутива Gradle. Исторический spike не изменяется.

JNA обновлена отдельно до 5.17.0: 5.16.0 проходит ELF/ZIP-проверку выравнивания,
но падает в `JNI_OnLoad` на API 36 с 16-КиБ страницами. Исправление описано
в [истории JNA](https://github.com/java-native-access/jna/blob/5.17.0/CHANGES.md).

```sh
rtk proxy rustup target add aarch64-linux-android
rtk proxy sh apps/taypeer-android/check.sh
```

`ANDROID_HOME` задаёт установленный SDK; по умолчанию используется `.tools/android-sdk`
в этом проекте. `GRADLE_USER_HOME` по умолчанию также локален `.tools/`.
`preBuild` собирает Rust, генерирует Kotlin тем же UniFFI 0.29.4 и собирает ARM64
библиотеку. Cargo использует общий lockfile. Генерируемые bindings, `.so`, SDK,
кеши и APK игнорируются Git. Android unit task пока не содержит JVM-сценариев;
платформенное поведение проверяют instrumentation-тесты.

Артефакты:

- `app/build/outputs/apk/debug/app-debug.apk`
- `app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk`
- `app/build/reports/lint-results-debug.html`

Проверка ELF всех `.so` и `zipalign -P 16` входит в `check.sh`. Выравнивание файла
не заменяет запуск на 16-КиБ образе.

Для запуска instrumentation на уже запущенном ARM64 AVD:

```sh
rtk proxy sh apps/taypeer-android/check.sh :app:connectedDebugAndroidTest
```

Для release задаются `TAYPEER_ANDROID_KEYSTORE` (вне репозитория),
`TAYPEER_ANDROID_STORE_PASSWORD`, `TAYPEER_ANDROID_KEY_ALIAS`,
`TAYPEER_ANDROID_KEY_PASSWORD`, затем `:app:assembleRelease`. Без этих параметров
release остаётся unsigned; debug key не становится ключом выпуска.
Это подготовка конфигурации подписи: nativeBuild пока использует Rust dev profile
для обоих вариантов. Подписанная оптимизированная release-сборка ещё не проверена.

## Матрица реализации

| Область | Текущий статус |
| --- | --- |
| Gradle → Rust ARM64 → Kotlin bindings | Реализовано |
| Отдельный UniFFI-crate, общий TOML, en/ru | Реализовано |
| Keystore credentials, проверяемый импорт ciphertext | Подключены к host, каталогу и SAF UI; без plaintext fallback |
| Isolated Service / смерть host / отдельный процесс | Настоящий документный worker, общий supervisor, независимый Binder death control |
| Дескрипторы и ciphertext staging | Реальный архив проверяется Rust внутри isolated UID; временные файлы приходят через Binder, позиционное I/O без путей |
| Генератор Compose → Rust, очистка UI, буфер | Пароли/фразы, параметры генерации, форма в памяти и блокировка результата |
| Независимые файловые БД, общий watchdog | DocumentManager удерживает отдельные supervised generations; сценарии документного worker и host death подключены |
| Каталог, создание, разблокировка, группы и записи | Подключены к общему Rust runtime |
| Пять вкладок, формы, история и оформление | Автосохранение, несколько форм, точное продолжение, tags, masked history, restore/purge, Lucide и RGBA |
| Вложения и SAF-return | Ограниченные выбранные потоки; сохранность после reopen, история/экспорт и scripted return проверены |
| Приглашения, direct/relay, JobScheduler и foreground exchange | Общий host; три Android-реплики, приём при блокировке и применение после разблокировки проверены |
| API 31 / API 36 / API 36 с 16-КиБ страницами | Итоговая матрица автоматизации ниже; исторические foundation-прогоны сохранены отдельно |
| Настоящее устройство, память и криптографическая приёмка | Открыто; эмулятор не закрывает эти условия |

Биометрия, камера QR, полный UI корзины и двусторонняя передача управления остаются
отдельными этапами Android-паритета. Обязательного разбора конфликтов нет; значения
выбирает общая детерминированная проекция, альтернативы доступны в истории.
KeePass, автобэкапы и восстановление резервных снимков исключены из релиза.
До закрытия аппаратной и криптографической приёмки используются только публичные
искусственные данные.

Основания платформенной границы: [Android isolated binding](https://developer.android.com/reference/android/content/Context#bindIsolatedService(android.content.Intent,int,java.lang.String,java.util.concurrent.Executor,android.content.ServiceConnection)),
[Keystore](https://developer.android.com/privacy-and-security/keystore),
[файловые операции](https://developer.android.com/reference/android/system/Os).

## Выполненные проверки

### Автоматизация — 2026-10-05

- API 31: восемь Compose-сценариев прошли, включая 500 мс автосохранение с сохранением
  фокуса, немедленную запись при переходе, реальное lock/reopen, независимые
  незавершённые формы, атрибуты/защиту, оформление/историю и scripted SAF-return.
- API 36 и API 36 PS16K (`PAGE_SIZE=16384`): по 31 платформенному сценарию подтверждено.
  На PS16K полный набор прошёл; на API 36 30 сценариев прошли полным набором,
  P2P отдельно повторён после исправления остановки обмена в teardown теста.
  Этот повтор также прошёл на PS16K; ошибки закрытия не игнорировались.
  Проверены документный worker, encrypted continuation, exact creation retry,
  отмена запуска до boot при уходе в фон, Binder ownership/CommitUncertain,
  foreground notification, bounded jobs, host death, Keystore, источник импорта,
  descriptor I/O, сохранность чужого буфера, вложения и P2P трёх реплик.
- Вложения проверены через настоящий worker: quota-before-read, источник,
  выросший после объявления размера, отказ замены без потери старого содержимого,
  encrypted incomplete form, reopen, retained history и точный durable export.
  Отказ доступа не открывает и не обрезает выбранный выходной файл.
- У занятого worker доступ отозван сразу, вызов background завершился менее чем
  за 500 мс; запрос принудительного завершения наблюдался примерно через 2000–2020 мс.
  Выход процесса подтверждался отдельно: это не жёсткая гарантия планировщика ОС.
- Android `check.sh` прошёл: APK/bindings, lint (0 ошибок, 11 существующих
  предупреждений), три ARM64 ELF и ZIP с 16-КиБ выравниванием. В release manifest
  есть приватные продуктовые services и нет debug probes/selected provider.
- Итоговый production APK этих проверок: SHA-256
  `7cdb4be30682c1e06c75fcd315c83c7a05fbb7727ba01203b088273c189866c8`.
  Последующие изменения instrumentation APK касались только наблюдения
  асинхронных вкладок и порядка завершения P2P-теста.
- Общие descriptor-process регрессии подтверждают приём ciphertext во время KDF
  при неизменных полномочиях и отказ после rotation до/во время credential read.
- Новые Native регрессии подтверждают CommitUncertain после durable commit при
  отказе передачи snapshot, сохранение категорий до commit и освобождение leases.
- Общий `RUST_TEST_THREADS=2 sh scripts/check.sh` прошёл полностью: модель,
  fmt/check/Clippy, workspace tests, 14 AppView-сценариев, Rustdoc, release workspace
  и headless smoke. Native Keychain tests штатно пропущены.
- Эта запись не закрывает физический Android, настоящий системный SAF picker,
  IME, TalkBack, память или криптографическую приёмку. Обычные значения и защита
  атрибутов редактируются с автосохранением; добавление/переименование строки и
  переименование вложения остаются явными адресными командами небольших форм.

### Исторический платформенный фундамент — 2026-09-22

- `cargo test -p taypeer-android --lib`: три Rust-теста прошли.
- `check.sh`: debug APK и instrumentation APK собираются; lint без ошибок.
  Предупреждения об обновлениях закреплённых библиотек и ChromeOS не являются
  заявлением поддержки этих версий/ABI.
- Все три native-библиотеки (Rust, JNA, Compose graphics path) имеют ARM64 ELF
  с выравниванием LOAD ≥16 КиБ; ZIP проверен `zipalign -P 16`.
- Общий `RUST_TEST_THREADS=2 sh scripts/check.sh` завершился успешно 2026-09-22: архитектура, fmt,
  check/clippy, workspace tests, 8 macOS UI-сценариев, rustdoc, release и smoke.
  Native Keychain tests штатно пропущены без `--native-keychain`.
  Проверен текущий Android-срез поверх command receipts и исправлений UI-тестов
  (`b9e36a5`). Первый прогон в sandbox не разрешил loopback-сокеты; при обычной
  параллельности отдельно падал favicon-тест, затем прошедший самостоятельно.
  Полный повтор вне sandbox с двумя тестовыми потоками завершился с кодом 0.

### Исправленные границы Android runtime

На первом ARM64 AVD API 31 выявлен отказ **первого** создания Rust host с
`ProfileBusy`. Прежний `NativeProfile` отображал любой отказ `File::try_lock` в этот
вариант. У стандартной библиотеки есть [известное ограничение Android file locks](https://github.com/rust-lang/rust/issues/148325).
Такой же вызов использовался в `taypeer-storage/src/file.rs` и
`taypeer-runtime/src/session/settings.rs`. Теперь `try_lock_exclusive`
в storage использует Android `flock` через rustix; runtime различает `Busy` и `Io`.
Тест первого host, отказа второму владельцу и повторного открытия профиля прошёл
на API 31, API 36 и API 36/16-КиБ.

Android SELinux также запретил hard link при импорте. Android-адаптер теперь
атомарно резервирует новый каталог рабочей копии через mkdir и публикует
проверенный staging через rename внутри этого каталога. Обе директории проходят
fsync; существующая рабочая копия не заменяется. Повторный импорт и отказ
провайдера проверяются независимо от будущего document-worker.

### Первоначальные прогоны до исправлений — 2026-09-22

ARM64 Google APIs images: API 31 revision 11 и API 36 revision 7.
На каждом выполнен набор из 8 instrumentation-тестов: 7 прошли, 1 отказ
(`rustHostUsesKeystoreCallbacksAndEnforcesSingleOwner`). Затем отдельно прошёл
добавленный девятый сценарий `configurationRecreationRetainsOnlyTheInMemoryForm`.
Итого на каждом образе 8 успешных сценариев и 1 незакрытый.

Подтверждены: Compose → UniFFI → общий генератор; очистка при уходе в фон;
сохранение формы при пересоздании Activity; ciphertext import и неизменность
источника; отказы провайдера/формата; Keystore roundtrip и отказ при порче;
независимые isolated процессы с загрузкой native; смерть isolated worker после
смерти host; буфер с восстановленной квитанцией и сохранением чужого копирования.
Это ещё не две открытые файловые БД и не приёмка документного worker.

Отдельный `native/examples/file-lock-probe.rs`, собранный тем же Rust для ARM64,
на обоих образах вернул `unsupported_or_failed: Unsupported`. Он использует
только std и новый публичный временный файл, не Keystore и не приложение.
Это подтверждает причину отказа до аутентификации/открытия БД.

На API 36 Google APIs PS16K revision 7 (`PAGE_SIZE=16384`) после обновления
JNA до 5.17.0 выполнены все 9 сценариев: 8 прошли, host profile lock вернул
`ProfileBusy`. Сбой `JNI_OnLoad` на JNA 5.16.0 больше не воспроизводится.
Прогоны API 31/36 с 4-КиБ страницами выше выполнялись на JNA 5.16.0;
результат повторной матрицы с 5.17.0 приведён ниже.

### Повтор после исправления runtime и файлового порта

API 31, API 36 и API 36/16-КиБ: на каждом все 10 тестов прошли, включая новый
`isolatedRustVerifiesTransferredArchiveUsingAnonymousCiphertextFiles`.
`CiphertextDescriptors` владеет полученными `ParcelFileDescriptor`; общий storage
читает и пишет через позиционный `CiphertextIo`, каждый FFI-вызов ограничен 64 КиБ.
Попытка открыть `/proc/self/fd` заново была отклонена isolated UID и удалена.
Проверка не использует fallback, доступ к host-каталогам или открытые временные файлы.
Debug acceptance service отсутствует в release manifest.
На API 31 и 36 затем отдельно повторён descriptor-сценарий с отказом allocator
и повреждённым архивом: оба отказа явные, прежний источник сохраняется при отказе staging.

Rust-тесты проверяют независимые смещения читателей, жизнь файла после unlink,
отказ выделения без fallback и сохранение прежней БД/редактируемого черновика при
ошибке staging; повтор той же операции после восстановления allocator сохраняется.
Это исторический результат платформенного фундамента; документный worker
подключён отдельно в текущем этапе автоматизации.
