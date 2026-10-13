# Приватный Android key pipe — PUBLIC spike

Стенд собирает **продуктовые** `BiometricKeyPipe.kt`, `BiometricOsKey.kt` и `biometric_key_pipe.cpp`
в отдельный APK `dev.taypeer.biometricspike`. Основной APK, его классы покрытия
и профиль не меняются. Граница предварительно задана
[контрактом биометрии](../../docs/android-biometry.md) и validated LikeC4 flows.
Native bridge и библиотека берутся из неизменяемого capture успешной продуктовой
сборки: `manifest.json`, `uniffi/` и `jniLibs/`, с SHA-256 каждого файла и APK.
Runner проверяет capture и отказывает при `--skip-build` с другим source/APK.

```sh
rtk proxy python3 spikes/android-biometric-pipe/check.py --serial emulator-5554 --product-native-capture artifacts/android/recovery-handoff-saf-host-covered-lint-producer-fixed/native-inputs --output artifacts/android/biometric-pipe-api31
```

Тесты используют настоящий JCA RSA/OAEP и настоящий JNI/private pipe. Все
ключи и 32-byte pattern специально созданы как PUBLIC fixtures. Проверяются
exact length/EOF, повтор, отмена, EPIPE, запрещённый файловый sink и исключение
провайдера. Gated JCA provider отдельно доказывает, что cancel закрывает FD/EOF
до возврата blocked decrypt, а поздний результат не публикуется.

Для OS-сценариев нужен специально зарегистрированный PUBLIC finger 1. Helper
работает только с явно выбранным ARM64 emulator API31+ без существующего PIN,
сохраняет boot-bound receipt и позволяет восстановить исходное lock-disabled
состояние. Он не изменяет физический телефон или чужой заблокированный профиль.

```sh
rtk proxy python3 spikes/android-biometric-pipe/enroll.py --serial emulator-5554 --state artifacts/android/PUBLIC-biometric-state.json
rtk proxy python3 spikes/android-biometric-pipe/check.py --serial emulator-5554 --product-native-capture artifacts/android/recovery-handoff-saf-host-covered-lint-producer-fixed/native-inputs --authorize-public-finger --output artifacts/android/PUBLIC-biometric-os-check
rtk proxy python3 spikes/android-biometric-pipe/enroll.py --serial emulator-5554 --state artifacts/android/PUBLIC-biometric-state.json --restore
```

OS tests используют настоящие auth-bound Keystore key и strong CryptoObject,
проверяют отказ нового Cipher без prompt, binding registry и JCA/Rust OAEP
совместимость. [Rust helper](rust-wrap/README.md) принимает только PUBLIC SPKI
и заворачивает фиксированные 32×`0x50`; пароль или произвольный ReadKey ему
передать нельзя. Runner доставляет только этот PUBLIC ciphertext тому же alias.
Маркер private APK может запросить лишь fixed finger 1 или rejected finger 2.
Точный `--test-class package.Class#method` позволяет независимый повтор; между
запусками останавливается только собственный PUBLIC APK.

На 2026-10-08 `biometric-os-rust-auth-api36-16k-platform-cancel-comparison`
подтвердил 12/12. API31 ранняя отмена AndroidX (`biometric-api31-early-androidx-cancel-independent`)
остаётся FAIL; самостоятельная отмена started sensor прошла 1/1, но общий набор
нестабилен. Эти отказы сохраняются как доказательства и не считаются PASS.
Настоящий isolated UID читает granted FIFO напрямую; `/proc/self/fd` reopen
отказал с EACCES. После перехода к прямому owned duplicate реальный Native importer
прошёл 13/13 на API31 и API36/16KiB (`biometric-safe-native-fd-api31`,
`biometric-safe-native-fd-api36-16k`). Exact 32-byte EOF, неверные длина/pattern,
file/socket/writer/nonblocking refusal и сохранность original проверены внутри
отдельного isolated UID. Positive JCA и Rust-wrap → OS CryptoObject → JNI → isolated
Rust, а также отказ нового unauthenticated Cipher, дали 3/3 на обеих платформах
(`biometric-safe-native-os-auth-api31`, `biometric-safe-native-os-auth-api36-16k`).
Положительные OS-тесты не читают ключ в instrumentation-процессе.

Отдельная продуктовая QA stage `--suite biometry --authorize-public-biometry`
проверила coordinator: actual cancel/EOF/dismissal и recreation без второго
authenticate — 2/2 на API31 и API36/16KiB. После stage исходные lock-disabled
состояния восстановлены и проверены по boot-bound receipts. Этот coordinator
ещё не подключён к полному enrollment/keyed-open пути.

Стенд пока не доказывает managed-open, registry enrollment, hardware security
level или memory acceptance. Только выполненные JUnit с exact APK/source hashes
являются результатом; ранние результаты не распространяются на новый source.
