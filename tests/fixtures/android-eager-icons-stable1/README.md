# PUBLIC eager70, схема 1

Все данные, пароль и seed искусственные и публичные. Пользовательские файлы
не использовались. Набор создан production Rust service API; он не является
захватом Android и не подтверждает native/UI-приёмку нового APK.
Отдельная лицензия набора не назначена; лицензия проекта остаётся пунктом roadmap.
Исторический [Android dev7 capture](../android-eager-icons/README.md) сохранён.

`seventy-image-entries.taypeer` содержит одну группу и 70 записей
`PUBLIC image 000`–`069`. У группы и каждой записи один локальный SVG 32×16,
114 байт; у каждой записи одна сохранённая версия. Незавершённых форм нет.
Пароль: `PUBLIC image list master`. Автор: `[71; 32]`, транспорт: `[111; 32]`.
Точные размер и SHA-256 закреплены в `manifest.json` и `SHA256SUMS`.

Генератор [eager_corpus.rs](../../../crates/taypeer-services/src/managed/tests/eager_corpus.rs)
использует создание БД, правку значка, Save и 69 настоящих clone-операций.
Генерировать только в новую директорию:

```sh
rtk proxy env TAYPEER_EAGER_CORPUS_OUTPUT=/private/tmp/taypeer-eager-new cargo test --locked -p taypeer-services --lib managed::tests::eager_corpus::generate_public_eager_corpus -- --ignored --exact
rtk cargo test --locked -p taypeer-services --test android_public_eager_fixture
```

Production RNG не подменён: повтор создаёт новый шифротекст. Обычные проверки
читают фиксированный файл, проверяют hash, все 70 изображений и историю,
запрещают запись и подтверждают неизменность ciphertext. Android отдельно
проверяет реальные worker, строки, пиксели и preview cache.
