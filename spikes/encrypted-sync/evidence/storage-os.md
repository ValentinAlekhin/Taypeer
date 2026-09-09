# Реальный отказ файловой системы macOS — 2026-09-09

Использован отдельный 32-МиБ HFS+ образ `/tmp/pass2p-storage-test.dmg`.
`disk-probe` создаёт только публичную синтетическую БД и filler внутри тестового
тома; размер filler ограничен 64 МиБ. Рабочий том нельзя использовать как аргумент.

Первоначальное создание (на повторном запуске не перезаписывайте существующий образ):

```sh
rtk proxy hdiutil create -size 32m -fs HFS+ -volname Pass2PStorageTest /tmp/pass2p-storage-test.dmg
```

Повторный прогон после реализации контейнера v3:

```sh
rtk proxy hdiutil attach /tmp/pass2p-storage-test.dmg -mountpoint /tmp/pass2p-storage-volume -nobrowse
rtk proxy spikes/encrypted-sync/target/release/disk-probe /tmp/pass2p-storage-volume full
rtk proxy hdiutil detach /tmp/pass2p-storage-volume
rtk proxy hdiutil attach /tmp/pass2p-storage-test.dmg -mountpoint /tmp/pass2p-storage-volume -readonly -nobrowse
rtk proxy spikes/encrypted-sync/target/release/disk-probe /tmp/pass2p-storage-volume readonly
rtk proxy hdiutil detach /tmp/pass2p-storage-volume
```

[Вывод обоих запусков](storage-os.log): `rejected_write=true`,
`previous_container_unchanged=true`. Filler получил настоящий errno ENOSPC.
Обе проверки завершились кодом 0; тестовый том отключён. В обычных тестах запись
на macOS использует fsync + F_FULLFSYNC и sync каталога; Android — fsync.
Отключение питания и поведение физических контроллеров не измерялись.
