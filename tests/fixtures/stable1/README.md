# PUBLIC-корпус схемы 1

Все значения, пароли и seed специально созданы Taypeer для тестов и публичны.
Пользовательские файлы и credentials не использовались. Отдельная лицензия набора
не назначена; лицензия проекта остаётся открытым пунктом roadmap.

Это первый кандидат стабильной схемы, а не подтверждение безопасности или выпуска.
Новые БД создаются со схемой 1. Исторические dev5/dev6/dev7 сохранены без изменения,
не читаются текущим Document и не мигрируются. Номер логической схемы независим
от внешнего контейнера и сетевого протокола.

- Контейнер: 1; компоновка архива: 5; документ: 1; control: 2; manifest/envelope: 1.
- Локальная коллекция черновиков: 2. Активная запись всегда имеет группу.
- Automerge: 0.7.4 из Cargo.lock.
- Обязательные read/write features: `taypeer.entries`, `taypeer.history`,
  `taypeer.lifecycle`, `taypeer.binary`, `taypeer.auto_merge`,
  `taypeer.required_group`, `taypeer.object_history`.
- Пароль БД: `PUBLIC_SESSION_DRAFT_PASSWORD`; автор `[19; 32]`, транспорт `[59; 32]`.
  Второй участник: автор `[20; 32]`, транспорт `[60; 32]`.
- ID рабочей копии sidecar: SHA-256 байтов `PUBLIC working copy`; скопированный
  `populated.draft` получает имя `<имя БД>.<hex ID рабочей копии>.draft`.

| Файл | Проверяемое состояние |
| --- | --- |
| `empty.taypeer` | Нет неявной группы или записи |
| `populated.taypeer` | Запись с паролем, защищённым атрибутом, вложением и историей; отдельные удалённый и очищенный объекты |
| `populated.draft` | Зашифрованная коллекция с незавершённой правкой `PUBLIC deferred corpus draft`; БД содержит `PUBLIC entry` |
| `conflicts.taypeer` | Выбранное название, альтернативы двух авторов, blob и неприменённый источник |
| `extensions.automerge` | PUBLIC-документ с неизвестными optional-полями и альтернативами неизвестного поля |

Пароль записи — `PUBLIC saved password`, защищённый атрибут — `PUBLIC protected value`,
вложение — точные байты `PUBLIC corpus attachment bytes` без перевода строки.
JSON фиксирует ID, поколения, поля, origins, историю, blobs, очистку и источники.
`SHA256SUMS` закрепляет входы; обычные тесты не обновляют их.

Генераторы: [service corpus](../../../crates/taypeer-services/src/managed/tests/corpus.rs)
и [document corpus](../../../crates/taypeer-document/src/tests/corpus.rs).
Из корня репозитория, только в новую директорию:

```sh
rtk proxy sh scripts/create-compatibility-corpus.sh /private/tmp/taypeer-schema1-new
rtk cargo test --locked -p taypeer-core -p taypeer-document
rtk cargo test --locked -p taypeer-services --lib managed::tests::corpus
rtk cargo test --locked -p taypeer-runtime --lib worker::tests::compatibility
```

Production RNG не подменён: повтор создаёт новые ID и шифротекст. Новая генерация
не заменяет этот фиксированный корпус и не закрывает native/память/KDF-приёмку.
