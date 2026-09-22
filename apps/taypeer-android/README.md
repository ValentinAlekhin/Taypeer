# Taypeer Android

Продуктовый Android-проект `dev.taypeer`, Android 12+ / ARM64. Цель — паритет
с macOS на `0104543`, с существующими ограничениями сервисов. Сейчас реализуется
платформенный фундамент; текущий экран позволяет только проверить генератор.
Открытие и редактирование БД и P2P ещё не подключены.

Точка продолжения: [состояние и следующие шаги](NEXT.md), 2026-09-22.

## Граница UniFFI

`native/` — отдельный crate основного Cargo workspace. Только он зависит от
UniFFI: типизированные DTO, ошибки, callbacks и преобразования. Общие crates
не зависят от Kotlin/Compose/UniFFI. Генератор вызывает общий сервис; проверка
импортируемого файла использует `RuntimeHost::inspect_compatibility`.
`Host` подключает защищённые credentials и остаётся владельцем ciphertext-runtime.
Его создание не разблокирует БД. Мастер-пароль в credential store не передаётся.

`DocumentService` — непубличный isolated service. Проверяется независимое связывание
через `bindIsolatedService`, загрузка native-библиотеки и завершение отдельного
процесса. Он пока **не обслуживает документ**: для этого необходимы descriptor-based
ciphertext persistence, передача авторских полномочий после аутентификации и
интеграция с supervisor. Подключать desktop spool paths или `open_local` в качестве
обходного пути нельзя. Смерть host отслеживается через Binder death recipient.

`KeystoreCredentials` хранит только AES-GCM ciphertext в `noBackupFilesDir`,
с service/account в associated data. Ключ остаётся в Android Keystore;
неподтверждённая запись и потерянный ключ — ошибки, без plaintext fallback.
Физическое затирание копий JVM/FFI этим не доказывается.

Импорт читает SAF только на вход, проверяет внутренний staging-файл общим Rust
reader, затем публикует новую рабочую копию без перезаписи существующей. Частичный
поток и неверный формат не попадают в каталог. Экспорт согласованного snapshot
координатора ещё не подключён.

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
| Keystore credentials, проверяемый импорт ciphertext | Реализованы адаптеры; экран импорта ещё не подключён |
| Isolated Service / смерть host / отдельный процесс | Есть lifecycle boundary и instrumentation-сценарий; документ ещё не подключён |
| Дескрипторы и ciphertext staging | Реальный архив проверяется Rust внутри isolated UID; временные файлы приходят через Binder, позиционное I/O без путей |
| Генератор Compose → Rust, очистка UI, буфер | Пароли/фразы, параметры генерации, форма в памяти и блокировка результата |
| Две файловые БД, KDF/запись/зависший вызов, watchdog | Не подключено на Android |
| Каталог, создание, разблокировка, группы и записи | Не подключено |
| Пять вкладок, черновик, история, вложения, оформление | Не подключено |
| Экспорт, SAF-return, lifecycle документа и inactivity | Не подключено |
| Приглашения, direct/relay, WorkManager и foreground exchange | Не подключено |
| API 31 / API 36 | По 10 из 10 сценариев прошли с JNA 5.17.0, flock и descriptor I/O |
| API 36 / 16-КиБ AVD | JNA 5.17.0: 10 из 10 сценариев прошли |
| Настоящее устройство, память и криптографическая приёмка | Открыто; эмулятор не закрывает эти условия |

Биометрия, KDBX, камера QR, UI корзины, разрешения конфликтов, отзыва и передачи
управления остаются следующим этапом. До закрытия аппаратной и криптографической
приёмки используются только публичные искусственные данные.

Основания платформенной границы: [Android isolated binding](https://developer.android.com/reference/android/content/Context#bindIsolatedService(android.content.Intent,int,java.lang.String,java.util.concurrent.Executor,android.content.ServiceConnection)),
[Keystore](https://developer.android.com/privacy-and-security/keystore),
[файловые операции](https://developer.android.com/reference/android/system/Os).

## Выполненные проверки

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
Это ещё не подключение документного worker к supervisor и координатору.
