# Серверные M3–M5 — реализованный контракт

Состояние на 26 ноября 2026. Новые возможности доступны через серверный HTTPS API;
оформление egui и браузерный клиент относятся к M5.5. Реализация и результаты
полевой приёмки разделены: [протокол проверок](m3-m5-verification.md).

## Хранение и исполнение

SQLite v10 добавляет библиотеку, задания загрузки, политики с историей, квоты,
workflow checkpoints, планы волн и курсор очереди. Перед обновлением существующей
v1–v9 создаётся отдельный consistent backup; старые бинарники новую схему не открывают.
DPAPI CurrentUser и права каталога сохраняются. Не переносить рабочую базу/токены
между identity и не менять учётную запись существующей службы без отдельного плана.

Два серверных worker и канал на 8 ID общие для Inventory, deploy, workflow и
библиотеки. Переполнение канала не теряет сохранённые задания: секундный диспетчер
повторно подаёт их из SQLite, продолжая сохранённый курсор и обходя весь backlog.
Одновременно действует максимум 128 активных library jobs и 128 незавершённых волн;
до 1024 приложений и политик, до 10 000 целей в плане волны. Полные lists ограничены;
отдельные задания/планы доступны по ID. Pagination/retention и production SLO — M6.

## Библиотека

Все новые endpoints доступны только Admin. Требуются Bearer token и действующий TLS.
POST очереди/плана требует `Idempotency-Key`: до 80 букв/цифр/дефисов. Повтор одного
ключа с другим содержимым даёт conflict. Редактирование объектов использует expected
revision; входной JSON ограничен 64 KiB, снимки — 8 MiB.

| Метод и путь | Действие |
|---|---|
| GET/POST `/v1/library/applications` | Список / `SaveApplication {application, expected}` |
| POST `/v1/library/jobs` | `Check {application_id, revision}` или `Download {release_id}`, tagged `kind` |
| GET `/v1/library/jobs`, GET `/v1/library/jobs/{id}` | Состояние, код, попытки, retry_at и release_id |
| POST `/v1/library/jobs/{id}/cancel` | JSON `null`, отмена только Queued |
| GET `/v1/library/releases`, GET `/v1/library/releases/{id}` | Кандидаты с evidence и результатом проверки |
| POST `/v1/library/approve`, `/v1/library/reject` | `{id, application_revision}` |
| POST `/v1/library/publish` | Одобренный release → отдельная immutable installer revision |

Источники: JSON Pointer, HTML selector/attribute, ограниченное выражение с именованной
группой `version`, либо явная manual version. Ссылка задаётся HTTPS template
`{version}/{arch}/{channel}`, извлекается регулярным выражением из links,
выбирается CSS внутри контейнера с точным заголовком либо задаётся вручную.
`LocalArtifact {sha256, extension}` использует уже добавленный на сервер файл.
Ноль/несколько результатов — ошибка, выбор «первого попавшегося» запрещён. Evidence
содержит время, SHA-256 ответа, конечный URL, redirects и число совпадений.

Состояния: `Candidate → Quarantined → Verified → Approved`, отдельно `Rejected`.
Скачивание и успешная проверка не запускают установку. SHA-256 сохраняется после
загрузки и больше не заменяется. Одобрение относится к точному hash/release/config
revision. Публикация требует проверенного локального артефакта и явного рецепта:

```json
{
  "release": {"id": "RELEASE_UUID", "application_revision": 1},
  "product_code": "UNINSTALL_GUID",
  "execution": {"kind": "Exe", "arguments": "/vendor-confirmed-silent"},
  "installed_version": "1.2.3.4",
  "allow_upgrade": false
}
```

Для EXE вместо GUID возможен точный `registry_key` в системном Uninstall;
wildcards, пути и пользовательские HKCU keys не принимаются. Параметры EXE не
угадываются. Для MSI execution — `{"kind":"Msi"}`. По умолчанию совпадает точный
DisplayVersion. Явный `version_components` у приложения допускает сравнение первых
N числовых компонентов, например публикация 7.1 и установленная 7.1.2609.301.
`allow_upgrade=true` разрешает замену только известной меньшей числовой версии;
неизвестная, более новая или отличающаяся форматированием равная версия блокирует
запуск. После выполнения всё равно требуется положительный detection; exit 0
сам по себе не означает успеха, 3010 не перезагружает компьютер.

Software matching использует точные name/publisher/architecture и при указании
ProductCode — сохранённое inventory evidence. Связь с ApplicationId/channel
вычисляется при чтении снимка: исходные scans не переписываются. Коллекции
отдельно сравнивают latest known и approved; пересчёт сам ничего не устанавливает.
Неизвестные или устаревшие данные сохраняют прежнюю семантику Unknown/Partial.

## Сетевые границы загрузчика

Только HTTPS, точный allowlist hosts/ports, до 5 redirects, без userinfo, cookies,
referer, системного proxy или передаваемых пользовательских auth headers. Каждый
redirect проверяется заново. Guarded DNS resolver проверяет реальные адреса при
соединении, включая IPv4-mapped IPv6. Private/loopback/link-local/metadata и
зарезервированные сети запрещены, кроме явного per-host CIDR исключения Admin.
Можно передать дополнительный CA PEM; проверки сертификата и имени не отключаются.

`trusted_proxy` — отдельный HTTPS proxy без пароля в URL. Это доверенный egress:
его администратор обязан обеспечить ту же DNS/IP фильтрацию на стороне CONNECT.
Сервер не может закрепить DNS-адрес назначения, разрешаемый внешним proxy.
Этот режим реализован, но не принят полевым proxy-тестом.

Ответ источника — до 2 MiB, артефакт — до 256 MiB, connect timeout 10 s, HTTP
attempt до 120 s. Компрессия ответа запрещена. `.part` возобновляется только со
strong ETag/If-Range, корректными 206/Content-Range и неизменным валидатором;
ответ 200 начинает файл заново. Проверяется хеш всего восстановленного файла.
Транзитные отказы получают максимум 3 сохранённые попытки с backoff/jitter;
для очень низкого лимита скорости может понадобиться отдельная загрузка крупного
файла и `LocalArtifact`: лимит времени не отключается.

Authenticode требует Valid и точного subject подписанта; альтернатива — явно
заданный SHA-256. До проверки файл находится в quarantine; cache адресуется хешем.
Из EXE читаются PE architecture/version resources, из MSI — properties/architecture,
без запуска файла. Невозможность чтения metadata запрещает автоматическое обновление,
но не подменяет результат отдельной проверки доверия.

## Workflow и восстановление

| Метод и путь | Действие |
|---|---|
| POST `/v1/workflows/revisions` | `{name,version,steps}` → immutable package revision |
| POST `/v1/workflows` | PlanDeployment + idempotency key → WaitingApproval |
| GET `/v1/workflows`, GET `/v1/workflows/{id}` | История/конкретный запуск |
| POST `/v1/workflows/{id}/approve`, `/retry`, `/reconcile`, `/cancel` | `{expected_revision}` |
| POST `/v1/workflows/{id}/resolve-step` | `{expected_revision,completed,evidence}` для неизвестного исхода |
| GET `/v1/deployments/{id}` | Полная сохранённая managed installation |
| POST `/v1/deployments/{id}/resolve` | `{expected_revision,confirmed_stopped:true,evidence}` → Failed |

Реализованы Install MSI/EXE, Command, PowerShell, FileCopy, Sleep, Message, Reboot,
Logoff, NestedPackage и SystemScan. NestedPackage разворачивается в закреплённые
ревизии при регистрации: до 16 уровней и 64 шагов, с проверкой циклов. Command
запускается без shell; PowerShell — явно заданный скрипт. Оба потока вывода
дренируются без сохранения. FileCopy проверяет хеш, подтверждает последовательные
64 KiB блоки WinRM, проверяет destination и заменяет файл атомарно.
Файлы для FileCopy добавляются локально на сервере:
`orkestr-server stage-file --data <DATA> --file <ABSOLUTE-LOCAL-FILE>`;
возвращённый SHA-256 указывается в шаге, cache extension — `.bin`.

Каждый шаг предваряется durable checkpoint. Ошибка/разрыв после границы запуска
даёт NeedsReconciliation и удерживает общую для deploy/workflow блокировку ПК.
После restart Running не переисполняется. Сверка installer/file/reboot/sleep —
read-only; произвольную команду автоматически повторять нельзя. Для неизвестной
команды Admin предоставляет отдельное evidence. Успешная сверка требует нового
одобрения перед следующим шагом; отрицательное ручное разрешение остаётся Failed.
Повторы допустимы только до запуска шага, максимум 3 на шаг. Отмена Running workflow
останавливает дальнейшие шаги после текущего; удалённый процесс не объявляется убитым.
SystemScan сохраняет и нормализованные Failed reports, связывает checkpoint с report
ID и безопасным кодом ошибки. Ошибка формата остаётся в checkpoint; Failed report
не заменяет последний успешный head и его дату.

`"account":{"source":"ServerIdentity"}` в `/v1/connections/setup` — явный вариант только Execution +
Kerberos. Он сохраняет DPAPI marker и использует identity процесса сервера без
запроса пароля. Скрытого fallback к текущему пользователю нет; Negotiate/рабочая
группа используют явные credentials. Для службы это будет именно её identity.

## Политики и волны

| Метод и путь | Действие |
|---|---|
| GET/POST `/v1/policies` | Список / `SavePolicy {policy, expected}` |
| GET `/v1/policies/{id}/history` | Неизменяемые ревизии политики |
| GET/POST `/v1/scheduler` | Квоты/leases / `{expected_revision,quotas}` |
| POST `/v1/rollouts/preview` | `{policy_id,policy_revision,generation,package_revision_id?}` → цели/волны |
| POST `/v1/rollouts` | То же с idempotency key → сохранённый план |
| GET `/v1/rollouts`, GET `/v1/rollouts/{id}` | Планы/полный снимок |
| POST `/v1/rollouts/{id}/approve`, `/pause`, `/resume`, `/cancel` | `{expected_revision}` |

auto-check, auto-download и auto-deploy независимы. Unattended — отдельное явное
opt-in; без него новая волна ждёт Admin. Политика закрепляет application, collection,
исходный проверенный рецепт и owner, interval/jitter, timezone window, pilot count,
soak, допустимый процент ошибок и max_attempts 1–3. Preview сверяется с generation;
созданный план хранит цели/профили/ревизии и не пересчитывает membership задним числом.

Auto-deploy следует только совместимым одобренным пакетам того же приложения и
ревизии конфигурации. При unattended сервер может одобрить строго более новую
Authenticode-verified версию с теми же name/publisher/architecture и проверяемой
версией metadata, сохранив прежние install arguments/detection/upgrade policy.
Смена MSI ProductCode меняет detection identity и требует нового явно выбранного
рецепта/ревизии политики; автоматический follow через другую GUID не допускается.
Новый hash при той же версии, другой рецепт, hash-only source, неизвестная metadata
или смена source revision требуют ручного решения. Произвольные workflows никогда
не получают unattended одобрение из одного auto-deploy flag.

Сначала выполняется pilot, затем soak, затем production. Error threshold,
NeedsReconciliation, отозванный/истёкший owner token, изменённая политика/источник,
сломанный последний source check останавливают новые старты. Уже начатый installer
может закончить работу. Retry_at сохраняется и ограничивает только безопасные
pre-install отказы; неизвестный исход не повторяется. Отмена родительской волны
запрещает дальнейший dispatch, отменяет ожидающих детей и запрашивает остановку
последующих шагов Running workflow. Сверка неизвестного результата остаётся доступной.

Окна используют IANA time zones, дни 0=Monday…6=Sunday, полуоткрытый интервал минут,
включая переход через полночь; UTC → local вычисление корректно обрабатывает DST.
Квоты global/site/download/transfer/install захватываются SQLite IMMEDIATE
transaction. Пустой site — `default`. По умолчанию global=2, per_site=1, downloads=1,
transfers=2, installs=2; strict_sequential даёт один общий слот. Install/Transfer
консервативно удерживают меньшую из этих квот на весь task, без промежуточного
освобождения. Повышение квот не создаёт больше двух worker в текущей реализации.

Скорость задаётся в **payload bytes/s**, 0 означает unlimited. Download budget
общий, endpoint budget делится на максимальное число одновременных transfers.
Новые настройки скорости применяются к новым передачам; снижение квот не прерывает
уже начатые задачи. Для смены общего потолка следует дождаться их завершения.
Это ограничение полезных данных с метриками bytes/elapsed_ms, не обещание wire Mbps:
TLS/WinRM/base64, buffering и системный трафик в него не входят. Точный лимит сетевого
интерфейса и нагрузочная приёмка нескольких физических площадок ещё не выполнены.

## Ассистент

[Готовый source config](../examples/library/assistant.json) отправляется в
`POST /v1/library/applications`. ID файла не зашит: ссылка извлекается из контейнера
с заголовком «Полная версия». Версия на официальной странице и версия PE различаются
по точности, поэтому явно выбраны 2 компонента. software_match publisher взят из
PE metadata; до production его надо сверить с реально установленной записью Inventory.

Этот конфиг разрешает проверку и загрузку. Установочный рецепт Ассистента не
публикуется: quiet bootstrapper не прошёл пилот. Не копировать исследовательские
параметры из локальных логов в политику. Подробности и оставшееся состояние SERVER01 —
в [протоколе](m3-m5-verification.md).
