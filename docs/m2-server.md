# M2 — общий сервер и консоли

Реализованный dev-контур, **не production-ready**. На доменном ПК начаты проверки
связности. Результаты и оставшиеся этапы описаны в [отчёте M2](m2-verification.md).
Членство Orkestr server/console в домене не требуется для HTTPS и персональных токенов.
Это не обещание, что Kerberos/Negotiate к AD заработает из любой workgroup-сети:
DNS, доступность DC, сертификаты, права и разрешённые протоколы проверяются отдельно.

## Границы

- `server`: HTTPS-only Rustls, SCM entrypoint и foreground для диагностики.
- `protocol` / `client`: отдельный контракт и blocking HTTPS на bounded UI worker.
- `credentials`: Windows DPAPI **CurrentUser**, SID, zeroizing buffers. LocalMachine
  fallback отсутствует. Другой SID / повреждённый blob блокируют startup **до listener**.
- SQLite v6 добавил режим WinRM, v7 добавляет планы и попытки Deploy, v8 — тип EXE
  и явные параметры запуска. Перед обновлением v7 создаётся `*.before-v8-<uuid>.sqlite`.
  Контракт установщиков и обновления: [m3-managed-installers.md](m3-managed-installers.md). Перед обновлением
  v6 создаётся `*.before-v7-<uuid>.sqlite`, v5 — `*.before-v6-<uuid>.sqlite`,
  v1–v4 — `*.before-v5-<uuid>.sqlite`. Старый сервер блокирует открытие новой схемы.
  Записи устройств, токены, DPAPI, задания и аудит сохраняются.
- Локальный NTFS, protected ACL: service identity + SYSTEM + Administrators. UNC,
  сетевые диски и reparse points запрещены. Single-instance file handle на Windows.
- Две и больше консолей читают один реестр, save/delete проверяют expected revision,
  конфликт — 409. Закрытие консоли не завершает сервер.
- Read-only jobs: 2 workers, очередь 8, 16 запросов одновременно, 128 запросов/сек
  глобально, body 64 KiB/10 секунд, HTTP client timeout 20 секунд, probe deadline 30 секунд.
  События возвращаются окном до 128; устаревший cursor явно требует resync.
  Это dev-лимиты, не согласованные production SLO/политики.
- Idempotency-Key сохраняется на `(actor_id,key)`. Повтор возвращает прежний job,
  другой connection с тем же ключом — 409. После restart Queued/Running → Interrupted,
  **без автоповтора**. Отменить можно только Queued; Running нельзя выдать за остановленный.

## Первый локальный запуск (без SCM и сетевых изменений)

Сборка из корня: `tools/check-core.ps1`, либо GNU-вариант из отчёта проверки.
После сборки — `target\debug\orkestr-server.exe`, `orkestr-console.exe`.
Команды ниже выполняются **под постоянной Windows identity будущей службы**.
Для первого локального теста допустим текущий пользователь. Для службы предпочтительна
выделенная локальная/доменная account с загруженным профилем; `runas /profile` и
«Log on as a service» настраивает администратор. Автоматического создания account нет.

```powershell
# Parent существует; каталог emp-m2-local ещё НЕ существует. Не использовать папку M1!
.\target\debug\orkestr-server.exe init --data C:\Temp\opencode\emp-m2-local --listen 127.0.0.1:7443 --dns localhost
.\target\debug\orkestr-server.exe run --data C:\Temp\opencode\emp-m2-local
```

`init` не перезаписывает каталог/файлы, создаёт пустой реестр и dev self-signed
сертификат. Не импортирует ПК, не ставит службу, не открывает порты, не меняет trust store.
При ошибке частично созданный каталог не перезаписывается повторным init:
сначала разберите причину, выберите **другой новый путь**; ничего не удаляется автоматически.

Откройте две консоли → **Подключиться к серверу…**:

1. Адрес `https://localhost:7443`.
2. **Загрузить DPAPI token…** → `bootstrap-admin.dpapi` из каталога сервера.
   Bootstrap действует 7 дней и доступен только исходному Windows-пользователю.
3. **Доверенный сертификат / CA…** → `server.crt`. Сверьте сертификат/отпечаток
   независимо; выбирать неизвестный файл как доверенный опасно.
4. В Настройках → **Операторы и аудит…** → Обновить метаданные → выдайте отдельный
   персональный token каждой консоли/оператору. Значение доступно только один раз.
   Для ручной защищённой передачи включите «Показать токен». На другой Windows identity
   нужен сам token, **не чужой DPAPI-файл**. Поле ввода не сохраняется в preferences.
   Выдайте рабочий Admin token до истечения bootstrap и храните доступ безопасно:
   проверенная offline break-glass recovery при истечении всех Admin tokens пока отсутствует.
5. Создайте группу одной консолью, обновите вторую (`Ctrl+R`). Одновременная правка
   одной revision должна дать conflict; draft остаётся в окне. Авто-refresh — 60 секунд.

Чтение private server SQLite через UI/CLI/SMB при работающем сервере не поддерживается.
Remote import файлов намеренно закрыт: клиентский path не становится серверным path.
Миграция существующего M1-реестра в серверную установку требует отдельной offline-процедуры;
не копируйте живой WAL или один `.sqlite` наугад.

## Роли и поля

| Роль | Доступ M2 |
|---|---|
| Viewer | Workspace/preview/health/audit, без write/jobs |
| Operator | То же + группы и создание read-only test job по известному connection ID |
| Admin | Профили, ввод секретов, выдача/отзыв токенов, чтение private импортированных данных |

M3.2 добавляет только Admin discovery preview/apply и safe Inventory status read для
Viewer+: [контракт и новое окно](m3-console-api.md). Старый remote **legacy import файлов**
остаётся закрыт; manual/CSV передаёт ограниченный текст, не клиентский filesystem path.

Approver/four-eyes/SSO ещё не реализованы. Роли проверяются **сервером**, а не только UI.
UI редакторов connections/credentials открыт Admin. Никакого endpoint произвольной
команды/installer/shutdown нет. Просроченный/отозванный token не работает, включая
queued job. Уже начавшийся bounded read-only test завершается; отзыв не выдаётся за kill.

Viewer/Operator не получают legacy private JSON (`scan.details`, `inventory_metadata`),
steps/metadata пакетов и import review: в них могли быть Environment/commands с секретами.
Нормализованные характеристики/ПО остаются доступны. Admin-ответы и вся БД **приватны**.
DPAPI ciphertext и пароли не возвращаются даже Admin через обычное API.

## Раздельные окна и read-only connection tests

**Настройки → Добавить подключение к домену или ПК…** открывает одну форму.
Для AD укажите DNS-имя домена и учётную запись чтения каталога. Сервер найдёт
контроллер по DNS, проверит доступность LDAPS и покажет выбранное имя после сохранения.
Контроллер также можно указать вручную. Для сканирования выберите WinRM и укажите
DNS-имя ПК и учётную запись с необходимыми правами. Название берётся из DNS-имени.

В карточке компьютера нажмите **Выбрать учётную запись…**. Сохранённый профиль
для этого DNS-имени используется повторно. Для зарегистрированного ПК также можно
выбрать доменную учётную запись из того же DNS-домена в формате `user@domain`,
если у неё есть права WinRM. Сервер создаёт профиль для выбранного ПК с Kerberos.
Список профилей загружается
при входе и обновляется после сохранения. Обычный запрос AD читает записи каталога,
а скан WinRM собирает актуальные данные с выбранного ПК.

Отдельные редакторы доступны в **Расширенных настройках сервера**.
**Учётные данные…** → **Обновить список** → **Новый профиль**. Укажите название,
AD Read или Execution, точный DNS-host, явный username и пароль. Scope — один host,
не wildcard/подсеть. Сервер сам шифрует пароль. При изменении выбирается актуальная
revision и вводится новый пароль; пустое значение не означает «удалить секрет».

**Подключения к ПК / AD…** → **Новое подключение**. Выберите профиль учётных данных.
Тип и host обязаны совпадать с credential scope; порт фиксирован безопасным baseline:

- **AD Read** проверяет RootDSE по LDAPS 636 с Negotiate. Для чтения выбранной OU
  используется отдельная команда в окне добавления компьютеров.
- **Execution** использует WinRM HTTPS 5986 и читает `Win32_OperatingSystem.Caption`
  через CIM. Выберите `Negotiate` для домена или рабочей группы. Для рабочей группы
  введите локальную учётную запись `HOST\user`. Режим `Kerberos` требует полного
  DNS-имени цели и доменной учётной записи. Ошибка Kerberos завершает проверку.
  В обоих режимах Windows на сервере проверяет TLS-сертификат WinRM.

Выберите сохранённый connection → **Read-only проверка с сервера** → обновляйте job.
Пароль передаётся фиксированному PowerShell adapter через stdin JSON, не argv/env.
Нет certificate-skip callback, Basic, TrustedHosts fallback или переключения на
service credentials. При смене revisions между enqueue/start test блокируется.
Вывод/exception удалённого хоста не пишется в аудит: только fixed safe code.
`CONNECTION_TEST_FAILED` остаётся общим кодом для ошибок DNS, TLS, аутентификации и CIM.
LDAPS без членства сервера в домене **не подтверждён** этим локальным тестом.

Для сетевого скана карточка различает недоступность порта 5986,
сбой TLS, отказ в доступе и ошибку открытия CIM-сессии. После сбоя сохраняется
предыдущий снимок ПК.

Явный режим сохраняется через `POST /v1/connections/winrm` с полем
`profile.winrm_authentication` (`Negotiate` или `Kerberos`). Правка через прежний
`POST /v1/connections` сохраняет уже выбранный режим. Профили без поля используют
Negotiate. Консоль блокирует запуск из несохранённой формы и передаёт `expected_revision`
в `POST /v1/jobs` для профилей с явно заданным режимом.

## SCM (только явный администраторский пилот)

Эти команды **не выполнялись** на компьютере владельца.

1. Под intended service account выполнить init и проверить foreground `run`.
   Bootstrap DPAPI/profile должен принадлежать именно этой identity, не установщику.
2. Остановить foreground. Скопировать собранный `orkestr-server.exe` в protected
   data directory; binary и TLS key тоже не должны быть доступны на запись console users.
3. Из elevated PowerShell, с явным service credential:

```powershell
$credential = Get-Credential # та же account, под которой выполнен init
.\tools\install-m2-service.ps1 -DataDirectory C:\ProgramData\OrkestrEMP -ServiceCredential $credential -WhatIf
# После проверки убрать -WhatIf: helper только регистрирует, не запускает службу.
Start-Service OrkestrEMP
Get-Service OrkestrEMP
# Stop-Service OrkestrEMP — только когда пилот требует остановки.
```

Путь в примере должен быть **тем же заранее созданным init-каталогом**, а не новой
пустой ProgramData-папкой. Helper не выдаёт Log on as a service, не настраивает recovery,
firewall, домен/WinRM и не меняет существующую службу. Service startup health проверяет
ACL/SID/profile, persisted canary, DPAPI round-trip, каждый сохранённый blob, БД и TLS key;
Running сообщается после bind HTTPS listener. Конкретный startup failure доступен в
foreground; интеграция детальных кодов в Windows Event Log ещё не сделана.

Для второй физической машины нужны сертификат с SAN реального server DNS-name,
узкое firewall-разрешение и явно заданный `listen` в protected `server.json`.
Текущий loopback/dev certificate нельзя выдать за готовую сетевую конфигурацию.
Автоматического trust/certificate enrollment нет. Revocation/CRL/OCSP hard-fail policy
пока не реализована: Rustls проверяет chain/name/expiry, но это не полная PKI-приёмка.

## Что ещё не закрыто

SCM start/stop/reboot под отдельной account; настоящий второй ПК/другая Windows identity;
AD/WinRM через сохранённые профили Orkestr; отрицательные реальные cert/auth
кейсы; PKI lifecycle/revocation; backup/restore/identity recovery drill; retention и
пагинация большого inventory. Workspace пока bounded snapshot (8 MiB), не scalable paging.
Preview M2 повторно оценивает freshness по серверному времени запроса; у снимка workspace
и следующего preview время может отличаться. На границе max-scan-age счётчики могут
отличиться до следующего refresh. Серверный pinned preview-session пока не реализован.
UI не production accessibility sign-off. Не включать M3 installs/agents/library скрыто.
Фактические тесты: [m2-verification.md](m2-verification.md).
