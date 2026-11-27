# M3.2 — ручной/CSV реестр в консоли и API, статусы Inventory

Продолжение [offline-ядра M3.1](m3-inventory-core.md), не завершение M3.
Нет новых AD/CIM/WinRM adapters, удалённых сканов и исполнения пакетов.
M2 domain/SCM/PKI acceptance остаётся открытой. Действующие функции M1/M2 сохранены.

## Добавить компьютеры

Собранная `target\debug\orkestr-console.exe` → открыть свою локальную базу либо
подключиться к HTTPS-серверу → **Компьютеры → Добавить компьютеры…**.

1. Выбрать **Ручной список** (DNS-имя в строке) или **CSV** (`host`, optional `name/site`).
2. Вставить текст или явно загрузить свой локальный UTF-8 `.txt/.csv`. Файл читается
   worker-потоком **на компьютере консоли**, не на сервере; файл не изменяется.
3. Нажать **Предпросмотр списка**. Показаны canonical host/alias → target host,
   DeviceId в подсказке, Add/Update/Unchanged/Duplicate/Conflict, старые → новые name/site,
   количество изменений, generation и срок preview. Registry пока не изменён.
4. Проверить весь список, установить checkbox подтверждения и нажать
   **Применить с резервной копией**. Это не разрешение скана, подключения или deploy.
5. При изменениях backend создаёт новый согласованный приватный backup рядом
   **со своей базой**, затем атомарно меняет registry и сохраняет receipt/audit.
   Имя backup видно в результате; remote-консоль не выбирает серверный path.
   No-op получает receipt без нового backup/generation. После успеха реестр обновляется.

В remote mode только **Admin** изменяет реестр; Operator/Viewer могут читать ПК и статусы.
Проверка серверная, до чтения body. Учётные данные вводятся только в прежнем отдельном
окне credentials M2; в discovery нет password/login/profile поля и скрытого connection test.

## Ограничения и подтверждение

- UI/API: **24 KiB UTF-8 текста**, до 1000 строк включая дубликаты. Не увеличен общий
  M2 body limit 64 KiB; escaped JSON также обязан поместиться в него.
- Offline CLI `discover` сохранил M3.1 limit 256 KiB и явный локальный backup path;
  это другая точка входа, не способ передать path серверу.
- Формат/identity/CSV semantics — как в M3.1. IP не принимается вместо DNS host;
  нет DNS lookup или short/FQDN guessing. Пустые CSV name/site сохраняют прежние значения.
- Неподдержанные headers/форматы/невалидный UTF-8/quoting дают ошибку без отражения
  raw source в ошибках. File reader отклоняет UNC/URL, non-regular file, прямой symlink
  и превышение размера; это не полная Windows mapped-drive/reparse-tree security review.
- Preview actor-bound: одна активная сессия на actor, **8 на Store**, TTL **5 минут**.
  Новый parsed preview отменяет прежний этого actor, в том числе при ошибке входа.
  Изменение draft в UI сбрасывает подтверждение/preview, старые token-ответы отбрасываются.
- Apply проверяет owner, confirmed, TTL и generation/devices baseline, затем проверяет
  freshness ещё раз под Immediate transaction после backup. Конфликт не перетирает
  изменения другой консоли. Неудача транзакции откатывает registry/receipt/audit вместе.
- Имя backup генерирует backend; существующие файлы **никогда не перезаписываются**.
  Ошибка после backup может оставить отдельную приватную копию, не доказывающую apply.

### Потеря ответа / restart

`preview_id` — durable idempotency key успешного apply, привязанный к actor. Повтор
того же запроса возвращает сохранённый receipt **без новой записи, backup и audit**,
даже после restart сервера и истечения preview TTL. Auth/expiry/revoke всё равно
проверяются: receipt не даёт обойти отозванный/истёкший token.

Неуспешно применённые previews живут только в памяти: после restart без receipt —
`PREVIEW_UNAVAILABLE`, нужен новый preview/подтверждение. Conflict также требует пересчёта.
Никаких автоматических retries нет.

При transport/неоднозначной ошибке окно сохраняет тот же preview ID и предлагает
**Повторить тот же apply / получить результат**. Нельзя молча изменить вход и отправить
новый ID. Закрытие окна не отменяет apply; pending ID остаётся в памяти этой консоли.
Закрытие процесса консоли/смена backend этот draft не сохраняет. Полный recovery UI
для потерянного ID/смены actor ещё не реализован; перед новым действием сверить registry/audit.

Apply — короткая операция локального registry на bounded backend request worker, не
network job. Backup большой БД может превысить client timeout (20 секунд): это **не**
доказательство отката. Использовать тот же preview ID для получения результата.
Большие volumes/SLO, cancellation и очередь для длительных imports остаются не приняты.

## Статусы в карточке ПК

Над вкладками показана **последняя попытка** Inventory, а её дата не выдаётся за дату
прежнего снимка. Если attempt не создал head, UI явно предупреждает, что ниже старые данные.

Вкладка **Провайдеры** показывает последнюю попытку и, если это другой report, провайдеры
сохранённого scan head: имя enum-раздела, Complete/Partial/Failed/NotCollected, число rows,
фиксированный failure code. Эти безопасные metadata доступны Viewer. При ошибке запроса
показана ошибка и доступна кнопка **Обновить статусы**; прежний скан не удаляется.

Данные запрашиваются on-demand для выбранного DeviceId; token/device/generation guards
не дают старому ответу сменить карточку нового ПК. No report для legacy import означает
«структурированных попыток M3 нет», а не успешный scan/пустой список провайдеров.
Вдобавок сохранены все прежние detail tabs; ComputerSystem/BIOS/OperatingSystem доступны
в соответствующих F21-разделах. `Все данные` по-прежнему приватны, Admin-only в remote mode.

Status response несёт generation того же read transaction, что latest/head. Если она
отличается от workspace, UI не смешивает новые provider states со старой карточкой,
а запрашивает refresh. При неудаче refresh ошибка остаётся видимой, без ложного успеха.

**Сетевых providers нет:** обычный legacy import не синтезирует новые M3 reports.
На настоящей старой базе вкладка честно покажет отсутствие структурированного отчёта,
пока будущие scan adapters не начнут сохранять reports. Тестовые данные — только fixtures.

## Реальный API этого инкремента

| Метод / endpoint | Роль | Смысл |
|---|---|---|
| `POST /v1/discovery/preview` | Admin | `{format:"Manual"|"Csv",text}` → prepared ID/rows/generation/TTL |
| `POST /v1/discovery/apply` | Admin | `{preview_id,confirmed:true}` → durable receipt |
| `POST /v1/discovery/discard` | Admin | JSON `null`, удалить pending draft этого actor |
| `GET /v1/devices/{DeviceId}/inventory-status` | Viewer+ | last/head safe metadata; неизвестный ПК → 404 |

DTO входа `deny_unknown_fields`: path, backup, rows, owner, arbitrary commands не принимаются.
Owner берётся из auth. Ошибки: 400 invalid JSON/fields; 401 auth; 403 role;
409 `CONFLICT` / `PREVIEW_UNAVAILABLE`; 422 `INVALID_DATA`; 503 `DISCOVERY_BUSY`.
Общие M2 body/request/rate/output limits действуют. Status ответ не содержит raw rows,
Environment, SID software scope, secret/ciphertext, exceptions или private filesystem path.
Raw report read/write endpoints не добавлены.

## SQLite v5 и эксплуатация

v5 добавляет только `discovery_receipts` (preview UUID, actor UUID, safe receipt JSON).
При writable open v1–v4 — consistent non-overwriting `*.before-v5-<uuid>.sqlite`;
read-only v1–v5 поддержаны. Scan reports/heads, secrets/canary, tokens/jobs, legacy imports
не переписываются. Старый v4 binary не откроет v5; действующую службу автоматически
не обновлять. Upgrade/rollback binary+DB — отдельная процедура на копии.

Backups содержат всю приватную базу, в том числе защищённые secrets. У сервера файлы
создаются в уже защищённом data directory; не публиковать их и не открывать по SMB.
Local mode требует собственного приватного локального каталога. Retention/cleanup
receipts, report history и orphan backups ещё нет; production disk-full/restore drill не принят.

В M3.3 появился [выбор режима WinRM HTTPS](m2-server.md#раздельные-окна-и-read-only-connection-tests).
Дальше идут обнаружение компьютеров по OU и задания инвентаризации F21.
Фактические результаты: [m3-console-api-verification.md](m3-console-api-verification.md).
