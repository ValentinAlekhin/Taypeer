# Android Compose → UniFFI → Rust

Изолированный UI-spike по [решению о двух фронтендах](../README.md).
macOS остаётся на GPUI + Kit. Этот workspace не входит в продуктовый workspace.
Вводите только публичные искусственные строки, например `SYNTHETIC-ONLY-Жук-42`.
`open_sample` создаёт тестовую сессию, **не проверяет мастер-пароль**.

**2026-09-09:** APK собран, 7 Rust-тестов и 7 инструментальных тестов на ARM64
эмуляторе Android 12 прошли. Приёмка реального устройства остаётся открытой.
Подробности, доказательства и ограничения — в [REPORT.md](REPORT.md).

## Сборка

Проверенное окружение: Temurin 17.0.20.1+1, Rust 1.97.1, Gradle 8.11.1,
AGP 8.9.2, Kotlin 2.1.20, SDK 35, Build Tools 35.0.0, NDK 27.2.12479018.
Минимальная ОС приложения — API 31 / Android 12, ABI — только ARM64.
Точные Rust-зависимости закреплены в Cargo.lock, Android — в
`app/gradle.lockfile`; wrapper проверяет SHA-256 дистрибутива Gradle.

На macOS с установленными JDK 17 и Android Command-line Tools, из этой папки:

```sh
export ANDROID_HOME="$PWD/.tools/android-sdk"
export GRADLE_USER_HOME="$PWD/.tools/gradle-cache"
sdkmanager --sdk_root="$ANDROID_HOME" --licenses
sdkmanager --sdk_root="$ANDROID_HOME" 'platform-tools' 'platforms;android-35' 'build-tools;35.0.0' 'ndk;27.2.12479018'
rustup toolchain install 1.97.1 --profile minimal
rustup target add --toolchain 1.97.1 aarch64-linux-android
sh build.sh
```

SDK-лицензии подтверждаются человеком; скрипт их автоматически не принимает.
Можно использовать существующие `ANDROID_HOME`, `JAVA_HOME`, `GRADLE_USER_HOME`.
В сессии с RTK перед командами используется `rtk proxy`.

`build.sh` проверяет Rust, генерирует Kotlin из только что собранной host-библиотеки,
собирает ARM64 `.so`, копирует её в jniLibs, собирает APK приложения и тестов,
запускает lint и проверяет упаковку библиотек. Генерируемые Kotlin, SDK/NDK,
кэш Gradle, `.so`, APK и debug-ключи не включаются в Git.
После изменения Rust используйте этот скрипт, а не только Gradle: прямой запуск
Gradle не обновляет UniFFI-привязки и `.so`.

Артефакты:

- `app/build/outputs/apk/debug/app-debug.apk`;
- `app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk`;
- `app/build/reports/lint-results-debug.html`.

Это debug APK для проверки; сборка не обещает побитового совпадения APK с другим
локальным debug-ключом. Версии и порядок генерации воспроизводимы.

## Запуск проверок

Подключите ARM64 Android 12+ с USB debugging или запустите ARM64 AVD, затем:

```sh
"$ANDROID_HOME/platform-tools/adb" devices -l
sh build.sh :app:connectedDebugAndroidTest
```

Команда устанавливает приложение и тестовый APK на доступные устройства.
Если устройств несколько, выберите одно через `ANDROID_SERIAL`.
JUnit/HTML-результаты находятся в `app/build/outputs/androidTest-results/connected/`
и `app/build/reports/androidTests/connected/`.
Эмулятор дополняет проверку, но не закрывает условие реального Android.

## Что проверяет стенд

Rust владеет искусственным образцом, проверяет поколение сессии и закрывает доступ
при блокировке. Kotlin хранит только данные, нужные форме, и отбрасывает ответы
старого поколения. Уход из Activity, включая системные диалоги, блокирует сессию.
Старое событие ввода не может восстановить поле после блокировки. Тексты не сохраняются
в Bundle; сохраняются только язык, режим темы и размер шрифта.

`palette.toml` читается общим Rust-кодом: напрямую на macOS, через UniFFI на Android.
Это тестовые палитры, а не утверждение о завершённом продуктовом оформлении.
Есть en/ru, три режима темы, размеры 14/16/18, прокручиваемые записи, защищённое поле,
диалог и платформенные упражнения SAF, clipboard и BiometricPrompt/AndroidKeyStore.
Последние требуют отдельного прохождения [матрицы](REPORT.md#оставшаяся-матрица).

Импорт принимает только файл до 1 МиБ с префиксом `SYNTHETIC-ONLY` и создаёт
внутреннюю рабочую копию. Экспорт пишет отдельный публичный образец. Ни один путь
не реализует формат БД. Биометрический тест использует отдельный тестовый ключ,
а не ключ реальной БД. Не используйте стенд для хранения паролей.
