# Android: локальная биометрическая разблокировка

Продуктовые enrollment, отключение и keyed unlock подключены к Native Host,
приватному OS adapter, isolated worker и Compose UI. Этот документ задаёт
границы ключей, FD, долговечной настройки и отмены; наличие кода не подтверждает
полную приёмку. [Результаты и frozen APK](testing.md) фиксируют целевой набор 6/6,
включая исправленный background-сценарий, отдельно от текущих изменённых исходников;
оставшиеся условия — в
[roadmap](roadmap.md#android-оставшиеся-проверки).

## Требование и владельцы

[spec.md](../spec.md#биометрия-и-восстановление-доступа) разрешает включить
биометрию после успешного ввода мастер-пароля, отдельно для БД на этом устройстве.
Она не сохраняет пароль, не переносится с файлом и не выдаёт сетевой допуск.
Отсутствие работающей биометрии возвращает к мастер-паролю. После ротации на
оставшихся устройствах нужны новый пароль и повторное включение биометрии.
Блокировка сохраняет требования [sessions.md](sessions.md): Android background
немедленно закрывает plaintext-сессию; поздний ответ не открывает её снова.

В Android макет «Биометрия · схема» в
[wireframes/source/android.js](../wireframes/source/android.js) задаёт контекст
конкретной БД, отмену и действие «Использовать мастер-пароль». Фактический запрос
аутентификации показывает ОС. Схема не требует собственной имитации системного
prompt или общей настройки биометрии для всех БД.

| Участок | Ответственность |
| --- | --- |
| Общий managed-сервис | Аутентификация текущего checkpoint ключом, authority/schema/admission guards и авторский handshake |
| Runtime worker | Отдельный keyed boot, enrollment proof и независимый контроль доступа |
| Native Android biometry adapter | Scoped registry, exact publication/retry и приватные input leases |
| `AndroidOsBiometry` / JNI broker | Auth-bound Keystore key, настоящий CryptoObject и передача ключа в принадлежащий worker FD |
| `DocumentManager` / Compose | Время жизни pending worker, локальный выбор БД, enrollment/disable/unlock и master fallback |

## Владение и доверенная граница

Рабочий вариант — auth-bound RSA keypair в Android Keystore UID приложения.
Isolated worker заворачивает текущий `ReadKey` публичным ключом. Только приватный
OS crypto adapter после настоящей аутентификации разворачивает эти 32 байта и
передаёт их напрямую isolated worker через отдельный FD.

Это осознанное расширение текущей ciphertext-only границы. Transient plaintext
допускается в узком native OS adapter в UID приложения; он не попадает в Kotlin
значение, UI DTO, Rust `RuntimeHost`, сеть или generic `CredentialPort`. Нельзя
обещать отсутствие временных копий внутри JCA/provider, отсутствие ключа во всей
RAM host UID или физическую очистку всех копий ОС.

| Владелец | Разрешённые данные и действия |
| --- | --- |
| Compose / экран разблокировки | Выбранная рабочая копия, публичный статус, действия Enable/Unlock/Cancel/Master password, локализованная ошибка |
| Приватный Android OS adapter | Keystore alias/opaque key handle, `Cipher`, настоящий `BiometricPrompt.CryptoObject`, публичные binding stamps; не возвращает plaintext |
| Приватный native broker того же UID | Одноразовый attempt, ссылка на Cipher, wrapped ciphertext, ограниченный native-owned direct buffer, запись ровно 32 байт в key FD и очистка собственного буфера |
| Rust `RuntimeHost` / supervisor | Публичная идентичность boot, ciphertext, opaque lifetime capability; не читает key FD и не вызывает decrypt для сети |
| Native isolated document worker | Единственный потребитель key FD, `ReadKey`, аутентификация текущего snapshot, существующий plaintext document/keyring |
| `CredentialPort`, `WorkerStreams`, Binder DTO и transport | Не получают `ReadKey`, мастер-пароль или plaintext direct buffer этого пути |

Keystore key остаётся у UID приложения; isolated service сохраняет отдельный UID.
Не нужны hidden Keystore grants, общий UID с worker или сохранённый мастер-пароль.
Ограничения доступа и свойства аппаратного исполнения описаны в
[Android Keystore](https://developer.android.com/privacy-and-security/keystore).

Для JNI/JCA handoff применяется минимальный private JNI/C++ broker: native
создаёт и удерживает direct buffer и вызывает публичный JCA API, Kotlin не
получает buffer как результат. Rust workspace сохраняет запрет unsafe;
JNI broker имеет отдельную узкую платформенную ответственность и не реализует
правила managed open, формат контейнера или криптографический алгоритм. Владение
буфером, Java global refs и FD имеет один RAII owner с очисткой при успехе,
исключении и отмене.

## OS key и совместимость OAEP

Схема локального wrapper: RSA-2048, exponent F4, OAEP с main digest
SHA-256, MGF1 SHA-1 и пустым label. Worker использует эти параметры явно; настройки
по умолчанию другой Rust/JCA библиотеки не считаются совместимыми. Android
рекомендует задавать оба digest через `OAEPParameterSpec`:
`RSA/ECB/OAEPPadding`, `SHA-256`, `MGF1`, `MGF1ParameterSpec.SHA1`,
`PSource.PSpecified.DEFAULT`.
[Официальное описание OAEP/MGF1](https://developer.android.com/privacy-and-security/cryptography).

Для private key задать decrypt purpose, RSA OAEP padding, SHA-256 digest,
`setUserAuthenticationRequired(true)`,
`setUserAuthenticationParameters(0, AUTH_BIOMETRIC_STRONG)` и
`setInvalidatedByBiometricEnrollment(true)`. Приложение имеет minSdk 31;
legacy timeout/device-credential ветвь не нужна. `setMgf1Digests` доступен с
API 35; на API 31 его не вызывать, совместимая MGF1 SHA-1 задаётся Cipher.
[KeyGenParameterSpec.Builder](https://developer.android.com/reference/android/security/keystore/KeyGenParameterSpec.Builder).

Не использовать OAEP label для root/epoch binding: проверенный публичный
[AOSP Keystore provider](https://android.googlesource.com/platform/frameworks/base/+/android13-mainline-art-release/keystore/java/android/security/keystore2/AndroidKeyStoreRSACipherSpi.java)
принимает только пустой label и MGF1 SHA-1. Binding обеспечивают отдельный scoped
alias, private registry и фактическая аутентификация точного текущего архива в
worker. Заголовок локального wrapper не является доказательством authority.

Prompt разрешает только `BIOMETRIC_STRONG`; OS PIN не подменяет этот способ.
Отрицательное действие возвращает к мастер-паролю Taypeer. Проверять доступность
через `canAuthenticate(BIOMETRIC_STRONG)`. При отсутствии sensor/enrollment,
инвалидации ключа или неподдерживаемых параметрах не создавать слабый ключ с
другими auth flags.
[Официальный biometric workflow](https://developer.android.com/identity/sign-in/biometric-auth).

Проверка реального ключа включает `KeyInfo`: authentication required, только
strong biometric, invalidation by enrollment и per-use duration **0 или -1**,
без положительного временного окна. Builder принимает timeout 0; фактический
Keystore2 на API 31 возвращает KeyInfo duration 0, хотя reference описывает -1.
Проверенная [AOSP factory Android 12](https://android.googlesource.com/platform/frameworks/base/+/android12-release/keystore/java/android/security/keystore2/AndroidKeyStoreSecretKeyFactorySpi.java)
инициализирует это поле нулём и непосредственно читает AUTH_TIMEOUT. Разница
представления не заменяет реальные проверки отказа без prompt и нового Cipher
после уже разрешённой операции. Security level и secure-hardware enforcement
фиксируются по фактическому значению, а не по названию устройства.
[KeyInfo](https://developer.android.com/reference/android/security/keystore/KeyInfo).

Public `Cipher.doFinal(ByteBuffer, ByteBuffer)` позволяет не получать секретный
`ByteArray` в Kotlin. Native output scratch ограничен размером RSA modulus,
256 байт; `getOutputSize` должен укладываться в этот предел. Returned length
должен быть ровно 32, после чего FD получает только 32 байта и EOF. Весь scratch
очищается при любом исходе; fixed 32-byte ReadKey не оправдывает игнорирование
JCA output-capacity contract.
[Cipher ByteBuffer API](https://developer.android.com/reference/javax/crypto/Cipher).

Эта схема защищает только локальный wrapper. Она не меняет формат БД, её KDF,
шифрование checkpoint, keyring или portable protocol. Результаты проверки
Rust-wrap → Android-authenticated-decrypt → isolated worker на API 31 и 36
хранятся в [testing.md](testing.md) отдельно от приёмки продукта.

## Scoped registry и публикация

Постоянная scope: namespace локального профиля, `WorkingCopyId`, `DatabaseId`,
проверенный authority root и текущая key epoch. Alias состоит из фиксированного
prefix, opaque scope digest и случайной key generation. Не включать название БД,
человеческое имя устройства, путь или строку alias из внешнего файла.

Запись version 1 в `noBackupFilesDir` содержит scheme, scope, key generation,
digest публичного SPKI и RSA ciphertext фиксированной длины 256 байт.
Лимит всей записи — 4 KiB; точная схема указана в
[контракте registry](#exact-registry-version-1). Alias не выбирается
из произвольного wrapper header. ReadKey или пароль в записи отсутствуют.
Публикация использует временный файл, fsync, rename и syncDirectory с существующим
различением definitive failure и `CommitUncertain`. Повреждённая/будущая запись
сохраняется, показывается ошибка и остаётся мастер-пароль; не выдавать её за
первое включение или молча заменять. Версия локального registry независима
от переносимого формата БД.

Обычное Save меняет checkpoint, но не должно отключать биометрию той же epoch.
Поэтому постоянная запись **не привязана к одному checkpoint digest**. Каждый
unlock attempt отдельно фиксирует immutable snapshot fingerprint, выбранный
checkpoint/baseline и current control head. Эти данные проверяются до prompt и
перед допуском сессии. Смена snapshot требует нового проверенного attempt;
wrapper старой epoch не получает исторический fallback для открытия новой epoch.

Keystore и файл не образуют общую транзакцию. Сначала создать новый key generation,
получить и проверить wrapper, затем durable publish pointer. При unknown
publication сохранить exact encrypted intent и key alias, повторять публикацию
или проверку того же результата. Не создавать второй ключ и не сообщать Enabled
до подтверждения. Cleanup удаляет лишь неподтверждённые orphan generations;
активный alias не удаляется по догадке после `CommitUncertain`.

## Включение после мастер-пароля

1. Пользователь явно включает биометрию для открытой локальной БД. Worker
   подтверждает, что эта текущая сессия аутентифицирована мастер-паролем, а не
   одной биометрией. Provenance — внутреннее состояние worker, не UI boolean.
2. OS adapter проверяет strong biometric/enrollment. Переход в OS Settings для
   enrollment действительно блокирует документ; возвращение требует нового
   мастер-пароля, а не сохранённого секрета из предыдущей сессии.
3. Создать auth-bound RSA alias для exact scope и передать только публичный SPKI
   текущему worker. Worker оборачивает текущий 32-byte ReadKey, не экспортируя его
   в host, и возвращает wrapped ciphertext с публичной scope.
4. Для подтверждения пригодности выполнить настоящий CryptoObject prompt и
   приватный unwrap/FD round-trip. Worker проверяет ключ на том же текущем
   snapshot; изменение generation/root/epoch прекращает включение.
5. Надёжно опубликовать exact wrapper record. Только этот ACK включает локальную
   настройку. Cancel/background закрывают attempt и его FD; late ACK не меняет
   экран другой БД. Публикация, уже начатая до отмены, остаётся предметом exact
   reconciliation, а не обещанием, что диск не изменился.

Enrollment wrap не становится общим `DocumentSession.wrapReadKey(publicKey)`:
такой метод позволил бы подставить произвольный ключ. Private OS adapter сначала
проверяет actual KeyInfo и создаёт одноразовую capability своего auth-bound alias;
только связанный private worker route принимает её public SPKI. UI не выбирает
ключ, алгоритм или destination для ReadKey. Та же capability связывает proof
prompt, wrapper result и final publication с одним alias и scope.

Не читать исторические epoch keys ради включения; не сохранять master input,
его производную или новый сетевой credential. Disable отдельно закрывает prompt,
отзывает alias и надёжно публикует локальное disabled состояние. Unknown disk
outcome не превращается в ложный UI ACK; отсутствие уже удалённого alias всегда
возвращает к мастер-паролю.

## Разблокировка и worker boot

1. Выбрать существующую внутреннюю рабочую копию через catalog. Новый импорт,
   другая копия и received ciphertext сами по себе не получают biometrics.
2. `DocumentManager` создаёт pending isolated process по существующим launch
   epoch/supervisor правилам. Worker читает ciphertext snapshot и проверяет
   публичную control/schema часть, возвращая public attempt challenge для
   exact copy/database/root/current epoch/snapshot. Приватный key FD отдельный.
3. Broker находит scoped alias, инициализирует новый decrypt Cipher с точными
   OAEP параметрами и запускает OS prompt с этим CryptoObject. Один attempt
   владеет одним Cipher; нельзя переиспользовать его для другой БД.
4. OS success callback ещё не означает открытие. Проверить attempt/Cipher identity,
   foreground, pending launch epoch, выбранную copy и generation. Private native
   adapter выполняет `doFinal` вне main/control thread, проверяет длину, пишет
   key FD и закрывает его. Отмена закрывает обе стороны FD; поздний result только
   очищается. Никакой JSON/ByteArray key response не создаётся.
5. Worker принимает строго 32 байта и EOF в ограниченный `ReadKey` owner.
   Узкий shared keyed-open повторяет выбор текущего baseline/checkpoint и
   аутентифицирует его этим ключом **до author callback**. Затем применяется
   существующая managed admission/compatibility/blob/draft логика и current
   authority recheck. Старый ключ не открывает learned rotation.
6. Успешный публичный session result регистрируется только при живом pending
   launch stamp. Накопленные ciphertext sources применяются по существующим
   Rust правилам после аутентификации. Ни prompt success, ни FD write ACK не
   открывают Compose экран сами по себе.

Key FD не добавляется к `PlatformDocument` как getter: этот контракт остаётся
ciphertext-only. Ключ не проходит через `WorkerStreams.read`, generic selected
input, `CredentialPort` или public UniFFI DTO. Binder передаёт только приватный
FD capability и публичный attempt stamp; потребляет его native entry isolated
worker, не Kotlin `DocumentService`.

## Lifecycle, отмена и инвалидация

AndroidX `BiometricPrompt` использует `MainActivity` на `FragmentActivity`,
с сохранением Compose/viewModels, закреплённой AndroidX biometric зависимостью
и разрешением `USE_BIOMETRIC`.
При конфигурационной recreation новый prompt owner переподключает callback к
тому же opaque attempt; второй `authenticate` не запускается. Cipher, ReadKey и
FD не сериализуются в saved state.
[BiometricPrompt lifecycle API](https://developer.android.com/reference/androidx/biometric/BiometricPrompt).

- Потеря focus сохраняет маскировку текущего UI; сама по себе не подменяет
  background и не отменяет ещё видимый OS prompt.
- Настоящий `onStop`, ручной lock, host/process death, смена БД или generation
  отменяют prompt через public API, инвалидируют boot stamp и закрывают key FD.
  Существующее двухсекундное независимое закрытие worker сохраняется, даже если
  Keystore или provider I/O ещё не вернулся. Поздний native result очищается.
- Back/Cancel/Master password прекращают биометрический attempt. Master input
  запускает новый обычный password boot после закрытия прежнего, не использует
  authenticated Cipher или ключ из отменённого attempt.
- При начале rotation/revoke текущий biometrics attempt приостанавливается.
  Definitive precommit rejection может сохранить прежнюю scope. При unknown
  исходе она недоступна до existing exact reconciliation; нельзя обещать, что
  старая epoch всё ещё актуальна.
- После подтверждённой или learned rotation новая root/epoch инвалидирует
  прежний record/alias. На оставшемся устройстве нужны новый master unlock и
  явное повторное включение; автоматического rewrap через старую биометрию нет.
- OS invalidation ключа требует мастер-пароль и повторное включение. Не
  пересоздавать alias с ослабленными auth requirements.

Проверка latest control выполняется в Rust, включая ситуацию, когда manifest
checkpoint ещё старый, а current baseline уже новый. Это не обещает удалённо
стереть незнающее об отзыве offline устройство или отменить доступ к старому
файлу с известным паролем. Биометрия не меняет эти свойства модели.

## Фиксированный OS port

OS port — доверенная платформенная граница, установленная **один раз при создании
application-owned Native Host**, до доступа UI. Конкретный private
`AndroidOsBiometry` владеет проверенными `BiometricOsKey` и реестром одноразовых
leases. Его нельзя заменить setter, передать заново в метод сессии или выбрать
из данных файла. Это доверие к владельцу Android adapter, а не криптографическое
доказательство из callback DTO с флагом `verified=true`.

`Host(directory, credentials)` остаётся совместимым входом без
биометрии. Bootstrap constructor
`Host.withOsBiometry(directory, credentials, osPort)` устанавливает private
владельца. Callback interface существует для платформенного bootstrap; Compose
и `DocumentState` не конструируют его и не передают SPKI. Rust Native хранит
того же владельца весь lifetime Host. Native registry принадлежит Android
biometry adapter, а не `RuntimeHost` или переносимому документу.

Ввод в `OsBiometryPort` всегда создаёт Native и проверяет по актуальному worker:

| Private platform call | Точный смысл |
| --- | --- |
| `availability()` | Available / NoHardware / NoEnrollment / Unavailable; не разрешает unwrap |
| `createKey(scope, attempt)` | Создать и проверить actual OS key, вернуть one-shot lease |
| `resolveKey(binding, generation, alias, spkiDigest, attempt)` | Проверить существующий alias той же scope, actual KeyInfo и SPKI, вернуть fresh lease |
| `runPrompt(call)` | Off-main ожидание одного private OS request; только Written / Cancelled / Unavailable / Failed, без session/Enabled ACK |
| `retireLease(lease)` | Независимо от provider отозвать одноразовый prompt/pipe owner |
| `deleteKey(binding, generation, alias)` | Удалить только exact scoped generation; отсутствие этой exact generation идемпотентно |

`OsBiometricKeyLease` содержит opaque lease ID, key generation, private alias
binding и публичный DER SPKI. Его возвращает только фиксированный port после
actual KeyInfo; Native проверяет форму, размер, exact scope/attempt и SPKI digest,
затем потребляет lease один раз. Lease не является UI DTO или сохраняемым
prompt state. Alias/DER не выбираются параметрами публичного действия. Во всех
Debug/error/logging представлениях DER и wrapped ciphertext скрыты.

`BiometricBinding` содержит namespace приватного профиля, working-copy ID,
database ID, root и epoch. Namespace — Native digest canonical private profile
directory; сам путь не входит в record или UI. `BiometricAttemptScope` добавляет
generation, случайный attempt ID, control head, snapshot fingerprint и выбранный
checkpoint/baseline. Она передаёт только публичные связи и не выдаёт author
authority. Permanent record не закрепляет checkpoint; каждый новый attempt
заново проверяет current selection общим Rust helper.

## Передача ключа через принадлежащий Native FD

Isolated UID не должен переоткрывать
`/proc/self/fd`: реальная проверка Android получила EACCES, хотя прямое чтение
уже переданного дескриптора разрешено. Используется существующая закреплённая
`filedescriptor = 0.8.3` через workspace. Safe
[`FileDescriptor::dup`](https://docs.rs/filedescriptor/0.8.3/filedescriptor/struct.FileDescriptor.html#method.dup)
создаёт собственный descriptor; `Read` и `AsFd` позволяют использовать уже
закреплённый `rustix = 1.1.4`. В исходнике этой версии Unix duplicate использует
`F_DUPFD_CLOEXEC`; библиотека, а не Taypeer, владеет необходимыми unsafe syscalls.

Private Native entry получает номер read FD только из удерживаемого
Binder-owned PFD. Маленький `HeldKeyFd` реализует `AsRawFd`, не `AsFd` и не
raw ownership conversion. Оригинальный PFD остаётся живым до завершения
синхронного `dup`. Уже собственный duplicate проверяется через safe
`rustix::fs::fstat` и `fcntl_getfl`: только FIFO, только read end, без
nonblocking режима. Ошибка закрывает duplicate; оригинал всегда остаётся
ответственностью PFD owner. Регулярные файлы, sockets, writable FD, отрицательный
FD и устаревший attempt не принимаются. После successful import original PFD
закрывается; sole Native duplicate закрывается один раз при drop. Повторный
`close` после EINTR не выполняется.

Только с `android-test-support` экспортируется
`verifyPublicBiometricPipe(fd: Int): Boolean`. Он дублирует и проверяет private
FIFO, внутри Native читает ровно 32 публичных байта `0x50` и EOF, сравнивает
фиксированный PUBLIC pattern и возвращает только результат. Ни массив байтов,
ни `ReadKey`, ни generic stream callback не возвращаются в Kotlin.
Standalone test вызывает именно собранный Native bridge в настоящем isolated
service. This support entry не входит в release ABI и не используется для
продуктовой разблокировки. Проверяются реальный JNI/provider→pipe→safe Rust
duplicate, EOF, неверная длина/pattern, file/socket/writer refusal и lifetime
после закрытия оригинала. Успех C++ pipe или обычного FileInputStream отдельно
не подтверждает этот Native путь.

## Enrollment и keyed boot

Native использует тот же узкий owned-FD helper в private worker entry. Обычные
`serveDocument` / password boot сохраняются wrappers; key FD не добавляется
к `WorkerStreams`, `CiphertextFiles`, `PlatformDocument` или JSON payload.
Платформенный input owner передаётся worker отдельно. Он связывает один FD с
ожидаемыми attempt/generation/scope, допускает одно потребление и закрывает
неиспользованный input при отмене. No shared plaintext cache создаётся.

Публичные действия Native имеют следующий контракт; OS prompt/FD wiring
принадлежит private platform coordinator, не Compose:

| API | Результат и guard |
| --- | --- |
| `Host.biometryStatus(copy)` | Disabled / Enabled / NeedsMaster / Pending; public binding, available action и pending operation; повреждённый/future record остаётся ошибкой |
| `DocumentSession.prepareBiometricEnrollment()` | Opaque `BiometricEnrollment` без public constructor; actual password provenance и fresh exact scope; SPKI не является аргументом |
| `BiometricEnrollment.identity()` | Только публичный `BiometricInputSpec`, проекция точной scope/lease |
| Private coordinator: получить prompt request и установить proof FD | Только fixed owner lease, wrapped RSA ciphertext и публичный attempt; ReadKey не возвращается |
| `BiometricEnrollment.confirm()` | Worker проверяет actual key из exact proof FD на том же current checkpoint и повторно проверяет password provenance; Native сохраняет exact record; Enabled только после durable ACK |
| `BiometricEnrollment.cancel()` | Немедленно retire lease/input и инвалидировать attempt; уже начатая unknown publication сохраняет encrypted intent |
| `Host.retryBiometricSetup(copy, operation)` | Только существующее proof-confirmed exact pending намерение; revalidate scope/actual OS key и повторно publish те же bytes/alias, без rewrap/new key |
| `Host.disableBiometry(copy, expectedGeneration, operation)` | Exact disable intent для выбранной публичной generation; revoke existing attempt/key, durable disabled record; unknown outcome требует exact retry |
| Private coordinator: begin biometric open(copy) | Pending supervised worker, public fresh challenge и separate input FD; никаких password/form/recovery/invitation parameters |
| Private coordinator: finish authenticated open | Только actual keyed worker session ACK для живого launch; OS success и FD write сами по себе не открывают UI |

Shared helper текущего scope переиспользует `current_checkpoint`; adapter не
копирует правила выбора baseline. Enrollment использует
`password_authenticated_key`, а proof аутентифицирует current checkpoint
полученным из FD ключом без изменения keyring/документа и без author callback.
Keyed boot использует `open_managed_with_key_and_author`; current scope
проверяется до аутентификации и непосредственно до author capability, а не
заменяется заявленной UI epoch. Смена snapshot/head/copy/root/epoch во время
prompt отклоняет attempt. В registry нет reusable author/session capability.

Native registry version 1, scheme `rsa_oaep_sha256_mgf1_sha1`, находится в
private profile под opaque copy-binding filename. Один bounded JSON record
содержит `active`, не более одного `pending` exact intent (Enable/Disable) и
одну latest operation receipt;
отсутствующие поля не подставляют успех. Record включает public binding,
случайную key generation, private alias binding, SPKI digest и ровно 256 байт
RSA ciphertext; pending также содержит operation и Native proof-confirmed stage.
Максимум **4096 bytes всего record**, до parse и до write. Exact IDs, alias
scope, lengths и version проверяются при чтении. Unknown/future/corrupt record
не заменяется default/новым включением. ReadKey, мастер-пароль, Cipher и FD
никогда не сериализуются.

Worker proof подтверждается **до** первой публикации proof-confirmed candidate.
Native сохраняет этот exact pending intent перед active transition. Publication
использует существующий `publish_file`: complete temp, file sync, atomic rename,
directory sync. После `CommitUncertain` память и durable candidate сохраняют
тот же operation/alias/ciphertext. Повторное чтение равных bytes не является ACK:
retry заново публикует/sync-ит этот record. До confirmed active publication
старый active alias не удаляется. Disable сохраняет exact намерение до OS
revocation, поэтому cold retry знает, какую generation завершать. Отказ до
publication не уничтожает старый active record; уже удалённый alias требует
master fallback и не выдаётся за подтверждённый Disabled.

Если epoch сменилась после actual worker proof и до подтверждения Enable,
старое exact намерение сохраняется. Retry сначала повторяет реальную публикацию
тех же pending bytes; потерянный ACK нельзя обойти новым ключом. Для той же
permanent namespace/copy/database/root разрешено отдельно завершить только
метадату старого proof-confirmed wrapper с исходными operation, epoch, alias,
SPKI digest и ciphertext. Это не подтверждает пригодность старого ключа:
статус для текущей epoch остаётся `NeedsMaster`, этот wrapper не запускает keyed
worker и OS prompt. Затем доступны exact Disable прежней generation либо новое
включение из текущего master-authenticated worker. Новый operation не подменяет
незавершённую публикацию; старые Enable/Disable retries не откатывают последующее
reenroll. Ошибка или future/corrupt record не превращаются в отсутствие настройки.

Native cancellation fence не зависит от AndroidX/SDK success callback или
фактической скорости скрытия системного prompt. Она сначала отзывает attempt,
закрывает private writer/input owner и отменяет pending launch, затем вызывает
OS cancellation. Late provider/result лишь освобождается и очищается.
Фактическое завершение OS cancel проверяется отдельно от Native fence.
Исторический API31 standalone FAIL и последующие coordinator/product результаты
относятся к своим checkpoint в [testing.md](testing.md); cleanup не превращает
неисполненный сценарий в PASS.

## Приватные интерфейсы

Интерфейсы реализуют связи `ffi → androidBiometry`,
`androidBiometry ↔ secretWorker` и `sessionGuard → androidBiometry`:
фиксированный OS port, отдельный key FD и независимый отзыв. `RuntimeHost`
не получает ключ; `PlatformDocument` не зависит от биометрии.

`BiometricAttemptScope { namespace, managed, generation, attempt }` проецирует
поля `BiometricBinding` и общего `ManagedKeyScope` один раз, реальное
`ProcessControl.generation` и случайный attempt digest. Поле `managed` содержит
`ManagedKeyScope`, который уже задаёт copy/database/root/epoch,
control, snapshot и checkpoint. `BiometricInputLease` содержит эту scope,
одноразовый input digest и закрытый `BiometricInputPurpose`:
`EnrollmentProof` либо `KeyedBoot`. Это публичные связи, не ключ и не разрешение
author. Их создаёт Native; UI не выбирает generation, input или SPKI.

| Private signature | Владелец и эффект |
| --- | --- |
| `run_platform_worker_with_biometry(reader, writer, document, inputs: Arc<dyn BiometricInputs>)` | Runtime worker получает отдельный приватный input owner; прежний `run_platform_worker` остаётся wrapper без биометрии |
| `RuntimeHost.begin_platform_biometric_open(launcher, BiometricBootAttempt { namespace, managed, attempt, input }) -> PendingBiometricOpen` | Тот же descriptor launcher запускает реальный supervised Opening worker; Runtime назначает actual generation и проверяет exact armed challenge до prompt |
| `PendingBiometricOpen.challenge() -> &BiometricInputLease`, `.control() -> WorkerControl` | Только public scope и независимый supervisor; pending не хранит FD/ключ и не удерживает command request во время prompt |
| `PendingBiometricOpen.complete(self, &lease) -> Worker`, `.close_report(&mut self) -> LockOutcome` | Written exact input завершает отдельную private boot-команду и обычную authority/application регистрацию; отказ или Drop отзывает Opening, close требует настоящий bounded OS/worker outcome |
| `serveDocumentWithBiometry(streams, persistence, files, selected, inputs: PrivateBiometricInputs)` | Native service bootstrap передаёт отдельный owner; исходный `serveDocument` сохраняет password/recovery/join контракт |
| `BiometricInputs.arm(lease)` | Rust-only вызов worker после actual scope validation; создаёт один ожидаемый slot, не экспортируется в Kotlin |
| `BiometricInputs.check(lease)` | Rust-only проверка независимого Native lifecycle fence до и после consume/read и непосредственно перед proof/author; это доступ, не OS-auth boolean |
| `BiometricInputs.consume(lease) -> Box<dyn Read + Send>` | Rust-only одноразовое получение owned input; возвращённый `Read` читает только isolated worker |
| `BiometricInputs.retire(lease)` | Rust-only отзыв exact slot/attempt; повторный отзыв идемпотентен |
| `validate_biometric_spki(&[u8]) -> Digest` | Единый bounded canonical RSA2048/F4 parser для fixed Native OS owner и worker wrap; digest не доказывает KeyInfo/prompt/authority |
| `PrivateBiometricInputs.install(lease, fd: Int)` | Private service bootstrap синхронно дублирует удерживаемый original PFD и удовлетворяет только уже armed slot; не создаёт scope |
| `IDocumentProcess.installBiometricInput(scope: BiometricInputParcel, input: ParcelFileDescriptor)` | Отдельный private Binder вызов; service вызывает Native install до ACK и закрывает original в `finally` |
| `IDocumentProcess.revokeBiometricInput(scope: BiometricInputParcel)` | Независимый control вызов; host enqueue не ждёт Binder/provider или command pipe |
| `DocumentProcess.installBiometricInput(spec: BiometricInputSpec, fd: Int)` | Private launcher callback: held host FD дублируется в PFD и передаётся exact service; не читает ключ |
| `OsBiometryPort.runPrompt(call: BiometricPromptCall) -> BiometricPromptOutcome` | Fixed port ожидает private future off-main; OS prompt и JNI принадлежат этому owner |

`BiometricInputSpec` — точный типизированный Binder projection lease; поля
ограничены до transfer и проверяются Native после transfer. Он не содержит
wrapped ciphertext, SPKI, bytes ключа или произвольного пути. Число FD
используется только внутри синхронного install, не сериализуется в worker JSON.
Обычные `WorkerStreams`, `CiphertextFiles`, selected-file callbacks,
`PlatformDocument` и `CredentialPort` этот FD не принимают.

Generated Kotlin создаёт фиксированный service owner обычным конструктором
`PrivateBiometricInputs()`, не `Companion.new()`: Rust bootstrap использует
именованный UniFFI constructor `new`. Service вызывает `installInput(spec, fd)`
и `retireInput(spec)`; методы arm/check/consume остаются Rust-only.

`PrivateBiometricInputs` имеет один armed slot и bounded набор terminal attempts
одного process generation. Не более 64 разных attempts допускаются за lifetime
owner; terminal IDs никогда не вытесняются. После исчерпания лимита новый attempt
отклоняется, остаётся master-password fallback с новым worker. Случайные digests
не считаются монотонными, а один последний tombstone не разрешает повторить более
старый отозванный attempt. Фиксированный service owner не заменяется на каждый prompt.
`install` проверяет exact attempt/generation/scope/purpose, выполняет owned dup
вне ownership mutex и повторно проверяет slot после dup. Original живёт до
возврата; cancellation между проверками закрывает новый duplicate и не даёт ACK.
Принятый duplicate проходит описанные выше FIFO/read-only/blocking guards.
Повторный install/consume, чужая scope и retired attempt отклоняются. Cancel
**терминально отзывает** lease, даже если input ещё не установлен; этот attempt
нельзя заново arm или использовать для нового prompt. Новый attempt имеет новую
идентичность; retry proof-confirmed registry не создаёт новый FD.

В host `BoundDocumentProcess` создаёт `ParcelFileDescriptor.fromFd(borrowedFd)`
и удерживает этот duplicate весь синхронный Binder install. В service original
PFD удерживается до Native dup ACK. После ACK host/service originals закрываются;
sole worker-owned duplicate освобождается при consume/drop. Ошибка install
закрывает все ещё принадлежащие caller handles. Ни root numeric FD, ни lease
не принимаются от UI для выбора произвольного key input.

`BiometricPromptCall` — opaque Native object без публичного конструктора.
Его получает только фиксированный bootstrap OS port. Private methods:
`scope()`, `leaseId()`, `wrappedCiphertext()` и `installInput(fd: Int)`.
Ciphertext имеет ровно 256 байт; input target уже связан с exact process/lease.
Ни alias/SPKI getter публичного `DocumentSession`, ни `wrapReadKey` не появляется.
Такой target исключает повторную установку OS owner и циклическую регистрацию
`Host → DocumentManager → OS adapter`: Native уже удерживает нужный process.

Fixed adapter создаёт pipe, передаёт sole writer в JNI `BiometricKeyPipe`,
устанавливает held reader через private target и закрывает свои originals после
ACK. Затем он ставит exact `BiometricPromptRequest` в main-thread coordinator.
Actual success callback проверяет тот же `CryptoObject` и strong biometric type
и лишь ставит JNI unwrap/write на off-main executor. `runPrompt` возвращает
`Written` только после typed успешного JNI finish с 32 байтами и EOF;
`Cancelled`, `Unavailable` и `Failed` различают прочие исходы. Эти enum outcomes
не являются аутентификацией БД, Enabled или session ACK. Native проверяет ключ
существующим authenticated checkpoint path независимо от ответа port.

Во время OS ожидания **не удерживается worker command mutex**, input mutex,
main thread или supervisor/control lock. `consume` вызывается только после
`Written`: worker не начинает читать pipe, пока prompt/provider ещё ожидаются.
При ошибочном/зависшем writer bounded read всё же может ждать EOF; лимит 32 байта
ограничивает объём, а не время. Cancel сначала ставит Native attempt fence,
закрывает sole JNI writer и отменяет pending launch, затем ставит OS cancel.
Это освобождает ожидающий read через EOF, не дожидаясь provider callback.
Если worker/provider не завершился, действующий supervisor принудительно
закрывает process по прежнему сроку; срок не увеличивается ради биометрии.
Отдельный control path остаётся доступным во время proof/keyed dispatch.
После любого read scope/access повторно проверяются до publication/author.
Фактическое скрытие prompt и terminal callback проверяются отдельно: отсутствие
dismissal не становится PASS после Native fence или test cleanup.

### Worker phases и зависимости

Enrollment выполняется одной exact попыткой:

1. `prepareBiometricEnrollment()` получает live scope через
   `password_authenticated_key`; возвращённый ключ остаётся в worker и
   уничтожается после запроса. Fixed port создаёт и проверяет actual OS lease.
2. Private platform-only wrap request повторяет password provenance и exact
   scope guards. Worker создаёт только wrapper, arm-ит `EnrollmentProof` и
   хранит public attempt/scope/SPKI digest/ciphertext, без дополнительного
   долгоживущего `ReadKey`. Host получает opaque enrollment.
3. `confirm()` off-main вызывает fixed port и ждёт `Written` без command lock.
   Отдельный proof request потребляет exact input, повторяет password provenance,
   аутентифицирует actual current checkpoint полученным ключом и повторно
   проверяет scope. Proof не вызывает author и не меняет документ или историю.
4. Content-free worker proof ACK предшествует первой публикации pending Enable.
   Native публикует pending и затем active; только последний durable ACK
   подтверждает Enabled. Late UI callback после lock не открывает экран снова.

Keyed boot — отдельная взаимно исключающая ветвь `PlatformBoot`; password пуст,
form/invitation/recovery отсутствуют. Worker сохраняет **Opening**, выбирает
actual snapshot/current checkpoint общим Rust helper, проверяет public scope
и arm-ит `KeyedBoot` до возврата challenge. `SessionToken` и author ещё нет.
Native проверяет permanent binding/epoch registry и actual resolved OS SPKI,
затем выполняет private prompt. После `Written` exact complete-boot request
потребляет input и повторно проверяет current scope. Только
`open_managed_with_key_and_author` аутентифицирует архив; callback проверяет
exact snapshot/scope непосредственно перед выдачей author. Затем действующий
apply/open путь даёт session ACK, `Worker::from_client` и реальный OPEN.
Manager проверяет launch epoch до регистрации owner и доставки UI.
Challenge-only worker обслуживает `Lock` и завершение без притворного сохранения
несуществовавшего plaintext draft. Ошибка/Cancel не вызывает password fallback
внутри keyed boot: пользователь отдельно вводит мастер-пароль.

RSA wrapper размещается в isolated runtime worker, используя закреплённый
RustCrypto API, а не собственные RSA/OAEP операции. Private DER input ограничен
4096 байтами до parse; принимается ровно один DER SPKI без trailing bytes,
RSA modulus ровно 2048 бит и exponent F4. Используются
`RsaPublicKey::from_public_key_der` и
`Oaep::new_with_mgf_hash::<Sha256, Sha1>()` с `rand_core::OsRng`, пустым label,
ровно 32 байтами worker-owned ключа и результатом ровно 256 байт.
Malformed/другая длина/PKCS1/другие параметры отклоняются; данные не логируются.
Product Rust не использует RSA private key для unwrap: эту операцию выполняет ОС.
[Pinned OAEP API](https://docs.rs/rsa/0.9.10/rsa/struct.Oaep.html) и
[SPKI decoder](https://docs.rs/rsa/0.9.10/rsa/pkcs8/trait.DecodePublicKey.html).
Первичный локальный source этих версий и прошедший
`spikes/android-biometric-pipe/rust-wrap` подтверждают точный вызов.

Зависимости и точные ревизии закреплены в workspace и Cargo.lock.
Отдельный `spikes/android-biometric-pipe` сохраняет собственный workspace
и результаты проверки взаимодействия Rust OAEP, OS CryptoObject и Native FD.

### Exact registry version 1

Canonical bounded serde record содержит `version: 1`, фиксированный `scheme`,
`active: Option<WrapperRecord>`, `pending: Option<PendingSetup>` и
`receipt: Option<SetupReceipt>`. Допускается только один pending и один latest
receipt. Unknown version/fields, malformed types и несогласованные связи
отклоняются без замены исходника или нового alias.

| Тип | Поля и проверки |
| --- | --- |
| `WrapperRecord` | permanent binding, generation 32 lowercase hex, exact scoped alias, SPKI digest, RSA ciphertext |
| `PendingSetup::Enable` | operation 64 lowercase hex, exact candidate WrapperRecord; stage только `proof_confirmed` после actual worker proof |
| `PendingSetup::Disable` | operation, exact expected generation существующего wrapper; не выбирает заново текущий alias на retry |
| `SetupReceipt` | только latest operation, action Enable/Disable и exact generation; сохраняется после удаления pending |

Digest/opaque fields имеют exact canonical lengths. `DatabaseId` сохраняет
общий тип и действующую shared validation: непустые 1..128 UTF-8 bytes;
он не обязан быть hex, Android не вводит новый name/ID rule. Epoch — `u64`.
Alias точно `taypeer_biometry_v1_<binding digest 64>_<generation 32>`:
117 ASCII bytes, без имени БД или пути. SPKI не хранится: fixed resolver
возвращает actual проверенный DER, Native вычисляет и сравнивает digest.
`BiometricOsKey.registeredDigest` проверяет actual OS key по scoped alias
и ожидаемому SPKI digest; DER в registry не добавляется.

Ciphertext сериализуется [base64 STANDARD](https://docs.rs/base64/0.22.1/base64/engine/general_purpose/constant.STANDARD.html)
с обязательным padding: ровно 344 ASCII characters, decode ровно 256 bytes;
noncanonical alphabet/padding/trailing bits отклоняются. Ни JSON numeric byte
array, ни секретное поле не используются. Serializer имеет фиксированный
порядок typed fields; retry удерживает exact serialized bytes, не пересоздаёт
wrapper/OAEP randomness. Предел 4096 проверяется до allocation/parse и до publish.
Максимальный валидный active+Enable record с worst JSON-escaped shared DatabaseId
и `u64::MAX` обязан укладываться в предел; это отдельный regression, а не оценка
по обычному hex fixture. Debug для record/lease/call скрывает alias, DER,
wrapped ciphertext; error не содержит record/body.

Наличие равного active на диске после cold restart **не является durable ACK**.
Перед первоначальным Enabled Native подтверждает loaded exact bytes повторной
публикацией и file/directory sync. Final active публикация сохраняет latest
receipt: потерянный последний reply разрешается match operation+generation и
републикацией тех же bytes, даже если pending уже отсутствует. Старый operation,
не совпадающий с pending/latest receipt, даёт typed Stale/NoPending; никогда не
создаёт новый ключ и не откатывает более позднюю настройку.

Disable получает `expectedGeneration` из публичного Native status и проверяет
её Rust guards. Public generation не раскрывает ключ и не выдаёт authority.
Exact intent публикуется перед OS delete; повтор завершает ту же generation.
Даже после reenroll старый disable retry не удаляет новый alias и не публикует
Disabled для него. Потерянный final reply сравнивается с latest Disable receipt
и подтверждает exact record повторной публикацией. Corrupt/future/unavailable
registry сохраняется; отсутствие usable OS alias возвращает master fallback,
не превращая unknown disk publication в успешную настройку.

## Проверки и критерии приёмки

Все fixtures искусственные PUBLIC. Тесты проверяют actual OS prompt, actual
Keystore key use и actual isolated worker, а не ручной вызов success callback.

Значимые Rust/Native сценарии:

- Bounded record: максимальный shared DatabaseId с worst JSON escaping,
  `u64::MAX`, active+Enable+receipt укладываются в 4096; 4097/truncated/unknown
  version/invalid base64/alias/copy сохраняют исходные bytes и не создают OS key.
- Actual publication faults до rename и после rename/directory sync failure:
  прежний active сохранён, unknown держит тот же alias/ciphertext/operation;
  exact retry и cold final-receipt retry повторно sync-ят без нового wrapper.
- Completed Enable → Disable → reenroll: старый Enable/Disable operation
  не откатывает новое состояние, старый expectedGeneration не удаляет новый key.
- Actual owned pipes: install ACK только после dup; original close не теряет
  Native input, slot однократен, cancelled install race не подтверждается,
  0/31/33 bytes и transport error не вызывают author/registry mutation.
- Actual archive proof/current keyed boot: правильный PUBLIC OS key открывает
  тот же copy; wrong key/root/copy/profile/epoch и snapshot race отклоняются
  **до author callback**, main ciphertext/history/catalog count неизменны.
- Password enrollment provenance: keyed-open, learned rotation и reopened old
  epoch не позволяют завернуть ключ; текущий новый master-open разрешает exact
  fresh enrollment, без чтения исторических ключей.
- Opening challenge lock, proof cancel и background после OS write: fence
  немедленный, late reply не даёт OPEN/Enabled, writer EOF и process close
  подтверждаются отдельно; cold master fallback сохраняет корректный архив.

| Проверка | Наблюдаемое доказательство |
| --- | --- |
| OS auth policy | Actual KeyInfo flags; `doFinal` без auth не даёт 32-byte key; fresh Cipher после предыдущего успеха требует новый prompt |
| Cross-language OAEP | Worker RSA-wrap с явным MGF1 → реальный Android CryptoObject decrypt → actual worker current checkpoint authentication на API31/36 |
| Длина и владение | 0/31/33 bytes, truncated FD, лишние bytes, duplicate consumption и cancellation не открывают БД; owner закрывает FD и очищает весь native scratch |
| Enrollment и prompt | OS Settings enrollment, правильный/чужой finger, Cancel/Back/Master password; только strong auth-bound operation открывает actual session |
| Scoped authority | Другая DB/copy/profile/root/epoch/alias либо подменённый record не вызывают author callback и не открывают документ |
| Snapshot race | Смена current control/snapshot во время prompt не допускает поздний boot; новый attempt проверяет актуальное состояние |
| Lifecycle | Background/lock до и после auth, pending registration race, recreation, Binder death и зависший provider; нет late UI revival, worker close ограничен существующим сроком |
| Rotation/revoke | Definitive reject сохраняет прежний валидный scope; unknown блокирует его до reconcile; confirmed/learned epoch требует новый master и reenroll |
| Durable setup/disable | Ошибка до публикации и lost ACK после rename различаются; exact encrypted retry не создаёт второй alias/record и не даёт ложный Enabled/Disabled |
| OS invalidation | Изменение enrolled biometrics/lockscreen делает прежний key непригодным; fallback master не ослабляет flags |
| Документ и сеть | Enroll/unlock не меняют main ciphertext/history/catalog count; reopen работает; network receive не вызывает prompt/unwrap и не получает key |
| Граница секретов | Private APIs/FD ownership проверены по типам и fault tests; UI/generated DTO/streams не содержат ReadKey, мастер-пароль не сохраняется, secret logging отсутствует |

QA stage использует отдельные runner-owned ARM64 AVD API31/API36 и
16-KiB вариант, exact serial и приватные PUBLIC profiles. OS credential и enrollment
не изменяют на чужих или уже занятых AVD. Enrollment производится через публичную
Settings UI; finger events — через официальный emulator console во время реального
prompt. Очистка/восстановление относится только к собственному AVD. Отсутствие
enrollment или неисполненный key test не превращается в PASS.
[Официальные fingerprint console команды](https://developer.android.com/studio/run/emulator-console).

Эмулятор подтверждает OS policy/lifecycle и реальный auth-bound API путь, но не
доказывает TEE/StrongBox свойства физического устройства, OEM поведение, сенсор
или физическую очистку памяти. Отдельная аппаратная приёмка проверяет actual
security level, enrollment invalidation, lock/background и Keystore поведение.

Результаты owned-FD/OS-auth стенда и продуктовых UI-прогонов сохранены в
[testing.md](testing.md). После первоначального background FAIL целевой набор
6/6 на frozen APK подтвердил также исправленный переход после OS success.
Текущие исходники отличаются от capture, полная матрица остаётся открытой. Исторический
API31 early-cancel FAIL сохраняется как результат своего standalone checkpoint.
Актуальные команды и PUBLIC enrollment/restore — в
[Android QA runbook](android-testing.md).
