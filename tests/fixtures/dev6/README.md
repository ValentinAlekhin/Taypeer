# Публичный корпус dev6

Все строки, пароли и seed специально созданы кодом Taypeer для тестов и публичны.
Пользовательские файлы, credentials и внешние данные не использовались. Отдельная
лицензия набора не назначена; лицензия проекта остаётся открытым пунктом roadmap.

Это экспериментальная схема 6, не стабильный формат и не свидетельство приёмки
безопасности. Старый [dev5](../dev5/README.md) сохранён без перезаписи; его документы
не мигрируются и не открываются текущим reader. Подписанный шифротекст с неизвестной
схемой может проверяться и передаваться отдельно от чтения документа.

- Контейнер: 1; компоновка архива: 5; документ: 6; control: 2; manifest/envelope: 1.
- Automerge: 0.7.4 из Cargo.lock.
- Обязательные read/write features: `taypeer.entries`, `taypeer.history`,
  `taypeer.lifecycle`, `taypeer.binary`, `taypeer.auto_merge`,
  `taypeer.optional_group`, `taypeer.object_history`.
- Пароль БД: `PUBLIC_SESSION_DRAFT_PASSWORD`; автор `[19; 32]`, транспорт `[59; 32]`.
  Второй участник: автор `[20; 32]`, транспорт `[60; 32]`.
- ID рабочей копии sidecar: SHA-256 байтов `PUBLIC working copy`; скопированный
  `populated.draft` получает имя `<имя БД>.<hex ID рабочей копии>.draft`.

| Файл | Проверяемое состояние |
| --- | --- |
| `empty.taypeer` | Нет неявной группы или записи |
| `populated.taypeer` | Активная запись с паролем, защищённым атрибутом, вложением и историей; отдельные удалённый и очищенный объекты |
| `populated.draft` | Локальная зашифрованная коллекция с незавершённой правкой `PUBLIC deferred corpus draft`; переносимая БД содержит `PUBLIC entry` |
| `conflicts.taypeer` | Автоматически выбранное название, сохранённые исходные альтернативы двух авторов, blob и полученный неприменённый источник |
| `extensions.automerge` | Открытый PUBLIC-документ с неизвестными optional-полями и альтернативами неизвестного поля |

Пароль записи — `PUBLIC saved password`, значение защищённого атрибута —
`PUBLIC protected value`, вложение — точные байты `PUBLIC corpus attachment bytes`
без перевода строки. JSON фиксирует идентичности, поколения, выбранные состояния,
исходные варианты и origins, историю, blobs, отметки очистки и полученные источники.
`SHA256SUMS` закрепляет все входы; обычный запуск тестов их не обновляет.

Генераторы находятся в [service corpus](../../../crates/taypeer-services/src/managed/tests/corpus.rs)
и [document corpus](../../../crates/taypeer-document/src/tests/corpus.rs). Из корня
репозитория, только в новую директорию:

```sh
rtk proxy sh scripts/create-compatibility-corpus.sh /private/tmp/taypeer-dev6-new
rtk cargo test --locked -p taypeer-core -p taypeer-document
rtk cargo test --locked -p taypeer-services --lib managed::tests::corpus
rtk cargo test --locked -p taypeer-runtime --lib worker::tests::compatibility
```

Production RNG не подменён: повтор создаёт новые ID и шифротекст, а не одинаковые
байты. Корпус проверяет reader/writer и обращения через временные копии, но не
нативные события, Keystore/Keychain, физическую очистку памяти или работу на Android.
