# PUBLIC Rust → Android OAEP fixture

Самостоятельный workspace для проверки совместимости с настоящим Android
Keystore/CryptoObject и JNI key pipe. Он не подключён к workspace приложения,
не принимает `ReadKey`, пароль или произвольный plaintext. Единственный образец —
32 байта `0x50`, намеренно PUBLIC. Приватный RSA-ключ в unit tests генерируется
только для этого PUBLIC software fixture и не сохраняется.

CLI получает два пути: публичный DER SubjectPublicKeyInfo, экспортированный
Android spike, и **новый** файл для 256-byte ciphertext. Существующий output
не перезаписывается; ошибки не выводят пути, DER, plaintext или ключи. Чтение
ограничено 4097 байтами, допустимый SPKI — 1…4096 байт. Парсер требует точный
DER SPKI RSA, modulus ровно 2048 бит и exponent F4 (`65537`).

```sh
rtk cargo generate-lockfile --manifest-path spikes/android-biometric-pipe/rust-wrap/Cargo.toml
rtk cargo test --locked --manifest-path spikes/android-biometric-pipe/rust-wrap/Cargo.toml
rtk cargo clippy --locked --manifest-path spikes/android-biometric-pipe/rust-wrap/Cargo.toml --all-targets -- -D warnings
rtk cargo run --locked --manifest-path spikes/android-biometric-pipe/rust-wrap/Cargo.toml -- PUBLIC-spki.der PUBLIC-rust-oaep.bin
```

Root runner передаёт полученный ciphertext обратно тому же OS-generated alias,
а после настоящего strong-biometric CryptoObject prompt проверяет JNI FD:
ровно PUBLIC 32×`0x50` и EOF. Генерация Cargo.lock и все проверки выполняются
владельцем текущего общего QA прогона, чтобы не пересекать активные Cargo процессы.
Этот helper не включает биометрию, не хранит wrapper registry и не доказывает
current managed-open или физическую очистку памяти.

Параметры заданы явно: RSA OAEP main SHA-256, MGF1 SHA-1, пустой label.
В закреплённом [RSA 0.9.10 API](https://github.com/RustCrypto/RSA/blob/v0.9.10/src/oaep.rs)
используется `Oaep::new_with_mgf_hash::<Sha256, Sha1>()`, у которого label `None`.
Это соответствует явному Android `OAEPParameterSpec` из
[контракта биометрии](../../../docs/android-biometry.md).

Unit tests проверяют реальное RSA round-trip с точными параметрами и отказ
при другом digest/MGF/label, случайность двух ciphertext, пустой/oversized ввод,
malformed/truncated/trailing DER, PKCS#1 вместо SPKI, другой exponent и modulus.
Они не заменяют настоящий Android prompt; результат фиксируется отдельно по
фактически выполненным тестам и hashes APK/исходников.
