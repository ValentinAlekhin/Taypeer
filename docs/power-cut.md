# Проверка потери питания

Успешный `SIGKILL`-тест оставляет живыми ядро и его файловый кэш.
Инъекция I/O-ошибки проверяет обработку отказа, но сама по себе не теряет volatile
записи. Эти результаты нельзя записывать как power-cut PASS.

## Linux block replay

Начальный стенд использует настоящий `ArchiveStore` и Linux `dm-log-writes`.
Записи восстанавливаются на свежий образ только до выбранного ACK mark,
после чего файловая система проходит journal recovery и отдельный процесс
проверяет подписанный manifest, generation, fingerprint и содержимое объектов.
Обычные записи target журналирует после flush; FUA — после завершения операции.
Это модель потерянного volatile tail, описанная в
[документации ядра](https://docs.kernel.org/admin-guide/device-mapper/log-writes.html).

Нужна полноценная Linux VM или тестовый Linux host: root, loop devices,
`dm-log-writes`, ext4, `dmsetup`, `losetup`, `blockdev`, `mount`, `umount`,
`mkfs.ext4` и C compiler. Rust toolchain — закреплённый в репозитории.
Сборку Rust выполнять обычным пользователем; elevated runner получает готовый
helper. Физические block devices не принимаются аргументами.

Созданная на машине владельца `taypeer-powercut` — Ubuntu 24.04 ARM64 с лимитами
4 ГиБ памяти и 8 ГиБ диска. Команда запуска:

```sh
rtk proxy orb start taypeer-powercut
```

Однако ядро OrbStack `7.2.9-orbstack-00391-g2b298d05fbd4` не предоставляет
`dm_log_writes`, `/dev/loop-control` и доступный device-mapper. Этот стенд
там не запускается; нужны собственное ядро полноценной VM и указанные устройства.
`modprobe`/preflight должны завершиться успешно до подготовки образов.

В подходящей Linux VM из корня checkout, после установки перечисленных утилит:

```sh
sudo modprobe dm_log_writes
cargo build --locked -p taypeer-storage --example power_cut
git clone https://github.com/josefbacik/log-writes /tmp/log-writes
git -C /tmp/log-writes checkout 7b70d8a6863c5de30933d42a7672d35d01d2dc6c
sudo sh scripts/check-power-cut.sh \
  "$PWD/target/debug/examples/power_cut" /tmp/log-writes
```

На машине владельца команды запускаются через RTK; внутри новой Linux VM
отсутствие RTK не блокирует запуск. Runner проверяет исходники replayer по
revision и SHA-256, собирает только эти C-исходники и использует собственные
loopfiles с фиксированным суммарным размером 640 МиБ. Неподдержанная среда
возвращает код 2 до создания образов; это не PASS и не пропущенный успешный тест.
Вывод сохранять вне тестового диска вместе с commit, kernel и helper SHA-256.

## Границы начального набора

Проверяются подтверждённые create и replace текущего подписанного архива,
в том числе многоблочное зашифрованное содержимое. Данные искусственные;
семантика документа/Automerge не проверяется. ACK/oracle хранятся вне
тестируемого диска. Каждая точка восстанавливается из одной исходной базы,
а не из уже восстановленного предыдущего результата.

Это начальный набор, а не полная матрица. Дополнительно обязательны:

- Обрывы до ACK и внутри записи: старый либо новый полный результат,
  отсутствие ложного успеха и корректный повтор без дублирования истории.
- Prepared/accepted anchor, создание иерархии профиля, credentials, смена
  пароля/KDF, приглашение, recovery receipt и повтор неопределённого результата.
- Durable ciphertext ACK при блокировке, черновики/discard, история/blobs,
  каталог рабочих копий и конкурентный writer после аварии.
- Физические APFS на Mac, NTFS на Windows и storage настоящего Android:
  контроллер, torn writes, питание и особенности файловой системы.

Команды и результаты ограниченного независимого разбора принадлежат
[security review](security-review.md), актуальные незакрытые условия —
[roadmap](roadmap.md#безопасность-и-платформенная-приёмка).
