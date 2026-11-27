# M3.1 — offline-ядро инвентаризации

Исторические границы M3.1. Текущие UI/API и SQLite v5 описаны в [M3.2](m3-console-api.md).

Первый ограниченный инкремент M3, **не завершённый M3 и не production-ready**.
Владелец выбрал начать ядро M3 без подключений и установок. Приёмка M2 остаётся
открытой: [m2-verification.md](m2-verification.md). Существующие UI/API/jobs не заменены.

## Реализовано

- `crates/inventory`: чистые модели, ручной список/CSV → preview, проверка данных
  провайдеров → нормализованный immutable-снимок. Нет сетевого/PowerShell-кода.
- `crates/storage`: атомарный apply preview и запись отчёта вместе со снимком,
  отдельная история неудачных/неизвестных попыток. Нет выполнения jobs.
- `crates/cli`: явный локальный `discover`, dry-run по умолчанию, apply только
  с новым rollback backup. Это tooling, **не новое окно remote-консоли**.
- SQLite **v4**: additive `inventory_reports` и индекс. При writable open v1/v2/v3
  создаётся согласованный `*.before-v4-<uuid>.sqlite`, без перезаписи. Generation,
  devices/scans, группы, M2 secrets/canary/tokens/jobs не переписываются миграцией.
  Read-only v1–v4 поддержаны; новые методы отчётов требуют v4.

Старый бинарник M2 с поддержкой только v3 не откроет v4. Не подменять binary у
действующей службы: upgrade/rollback проводится отдельно, по плану миграции.

## Ручной список и CSV

Ручной UTF-8-файл: одно DNS-имя в строке; пустые строки пропускаются. Регистр и
конечная точка нормализуются. CSV: UTF-8, необязательный BOM, разделитель **запятая**,
обязательная колонка `host`, необязательные `name` и `site`; порядок/регистр заголовков
не важны. Пример **синтетический**, имена `.test` не являются тестовыми целями:

```csv
host,name,site
pc-01.example.test,"Учебный ПК, приёмная",Офис
pc-02.example.test,Учебный ПК 2,Офис
```

Лимиты: 256 KiB входа, 1000 записей включая дубликаты; name/site до 256 байт.
Quoted fields и escaped `""` поддержаны; неправильные/незакрытые кавычки,
неизвестные/повторные заголовки, разное число колонок и control characters в labels
отклоняются. Кавычки должны начинаться сразу после разделителя, без внешних пробелов.
CSV с `;`, XLSX, UTF-16, произвольными колонками/паролями не поддерживается.

- IP вместо DNS-имени отклоняется. DNS не опрашивается; short name и FQDN не
  объединяются догадкой. GUID не назначается по совпадению имени.
- Preview: `Add`, `Update`, `Unchanged`, `Duplicate`, `Conflict`, DeviceId и generation.
  UUID новых ПК назначаются на preview, не означают, что ПК уже сохранены.
- Совпадение по точному canonical host/alias сохраняет UUID. Ввод старого alias
  **не переименовывает** ПК. AD GUID, disabled, OU/domain/OS, metadata и скан сохраняются.
- CSV меняет только явно непустые name/site. Пустое поле не очищает сохранённое.
  Source существующего ПК не заменяется; у нового — `Manual` или `Csv`.
- Одинаковые строки — дубликаты; разные данные для одного host, два alias одного
  DeviceId и неоднозначное владение alias блокируют **весь** apply.
- Apply повторно проверяет generation и исходные devices под Immediate transaction.
  Чужой/stale preview не перезаписывает реестр. Ошибка откатывает все записи.
  No-op не увеличивает generation. Ничего не удаляется и не запускается.

### CLI

Использовать **свою локальную offline-базу/копию**, не private DB работающего сервера.
Входной файл также локальный. Remote import по API пока отсутствует.

```powershell
# База уже создана явным init; без --apply ничего не записывается.
.\target\debug\orkestr-cli.exe discover manual C:\MyCopies\orkestr.sqlite C:\MyCopies\hosts.txt
.\target\debug\orkestr-cli.exe discover csv C:\MyCopies\orkestr.sqlite C:\MyCopies\devices.csv

# Только после просмотра отчёта; backup path должен быть новым.
.\target\debug\orkestr-cli.exe discover csv C:\MyCopies\orkestr.sqlite C:\MyCopies\devices.csv --apply C:\MyCopies\before-discovery.sqlite
```

CLI удерживает разобранное содержимое, а не перечитывает изменяемый файл при apply.
Повтор команды строит новый preview. Парсер не выводит исходные строки в ошибках;
успешный preview содержит host, но не private metadata/сканы.

## Контракт отчёта провайдеров

`InventoryReport`: UUID, DeviceId, UTC started/finished, независимые `ProviderReport`.
Все **23** F21-раздела представлены в `InventoryProvider::ALL`: system/OS/BIOS,
CPU/RAM/NIC/disk/display/hardware, hotfixes, users/groups, printers/processes/services,
shares/environment/features/startup/tasks и machine/user software. Это контракт
данных, **не утверждение, что написаны 23 сетевых адаптера**.

Каждый провайдер имеет `Complete / Partial / Failed / NotCollected`, raw object rows
и необязательный фиксированный failure code. Пропущенные — `NotCollected`, а не пустой
успех. Failed требует code и не имеет rows; Complete не может иметь failure.
Дубликаты провайдера, некорректные times/rows и превышение лимитов блокируют отчёт.
Лимиты входного отчёта: 4 MiB serialized JSON, 10 000 rows на провайдера.

Общий статус: все Complete → Complete; есть Complete/Partial → Partial;
иначе есть Failed → Failed; иначе Unknown. Failed/Unknown сохраняются **без нового
DeviceScan**: прежний head и его дата не обновляются. Partial создаёт собственный
снимок с Unknown для отсутствующих данных, **не подклеивает** старые значения под
новую дату. Поздно завершившийся старый отчёт не вытесняет более новый scan head.
Свежесть снимка считается от `started_at`, а не от завершения медленного задания.

RAM: только полный непустой набор `MemoryModules[].Capacity` (integer bytes либо
decimal string), положительные значения, checked sum. Пропуск/ноль/overflow/Partial
→ Unknown, без подстановки округлённого MemoryGB; некорректный Complete становится Partial.

### ПО

Rows software-провайдеров: `name`, `publisher`, `version`, `architecture`,
`product_code`, `registry_key`; у user-provider дополнительно `user_sid`.
Нормализованные `SoftwareEvidence` сохраняют raw version, optional numeric normalized
version, arch, machine/user scope, product UUID и registry identity. Raw rows не теряются.
Пустое имя/невалидная структура/SID/product UUID понижают провайдер до Partial;
непонятная версия/архитектура остаётся Unknown, не выдумывается из имени продукта.
Numeric normalization: `v02.010.0` → `2.10`; prerelease пока Unknown (как в M1).

Убираются только точные дубликаты нормализованных evidence. Разные publisher, version,
scope, architecture, product code или registry key не сливаются. ApplicationId/channel
не угадываются: library matching — M4. Исходный M1 `InstalledSoftware` остаётся
совместимой проекцией, расширенная identity/scope лежит в private evidence.

Общий software state = Complete **только** при Complete machine **и** user providers.
Будущий user adapter вправе так заявить лишь при доказанном полном coverage всех
предусмотренных user scopes; чтение только загруженных HKU — Partial. Один HKLM
никогда не доказывает отсутствие/актуальность ПО. Старый engine групп сохраняет
трёхзначную семантику; version/channel/library Unknown не превращается в compliance.
Отсутствующий Publisher при фильтре по издателю теперь также Unknown, не доказательство
другого продукта/отсутствия ПО; известный другой publisher/identity не совпадает.

## Хранение и приватность

`record_inventory` принимает только результат normalize; report и optional scan пишутся
одной транзакцией. Повтор UUID — ошибка без частичной записи. `inventory_report(id)`
и indexed `latest_inventory_report(DeviceId)` читают private evidence. Последний отчёт
может быть Failed/Unknown при сохранённом старом удачном scan head: это разные сущности.
History paging/retention ещё нет; API/GUI индикации последней неудачи тоже пока нет.

Raw F21 rows остаются в sections `DeviceScan.details`, статусы и software evidence —
в `OrkestrInventory` schema 1. Эти данные/вся БД/backup **приватны**, могут содержать
Environment/command paths/персональные данные. Обычный M2 Viewer/Operator по-прежнему
не получает scan.details; новых public endpoints нет. Отчёт не исполняет raw строки.

## Не сделано / следующий инкремент

ADSI/OU discovery/paging и rename reconciliation на живом AD; CIM adapters F21;
сохранённые domain/workgroup WinRM profiles; scan job wiring/cancellation/restart;
новое окно подключения и provider-status UI/API; deployment locks/attempts/approval/
detection/reboot/reconciliation. Автоматического импорта/скана/установки нет.
Подключение новых моделей к UI/API — отдельный следующий инкремент, не скрытая замена M2.

Фактические проверки и Not run: [m3-inventory-verification.md](m3-inventory-verification.md).
