# Предлагаемая архитектура

Статус: целевая архитектура. M1 реализует `domain`, `storage`, `application`,
нативную `console` и диагностический `cli`; M2 добавляет `protocol`, `credentials`,
`client`, `server` и remote-mode. M3.1 добавляет offline `inventory` и SQLite v4 reports,
без сетевых адаптеров/новых endpoints: [m3-inventory-core.md](m3-inventory-core.md).
M3.2 связывает manual/CSV и safe provider status с application/client/server/console,
добавляет actor-bound preview/receipt в SQLite v5: [m3-console-api.md](m3-console-api.md).
M3.4–M3.6 добавляют сетевой Inventory и управляемые MSI/EXE; текущий контракт
установщиков: [m3-managed-installers.md](m3-managed-installers.md). Реальный контракт
и ограничения M2: [m2-server.md](m2-server.md), ниже также есть целевые будущие API.

M3–M5 реализованы на сервере: новые crates `library` и `scheduler`, SQLite v10,
immutable workflows, durable dispatcher/quotas/policies и payload pacing.
Текущий контракт и его пределы: [m3-m5-server.md](m3-m5-server.md),
полевые результаты: [m3-m5-verification.md](m3-m5-verification.md).

## Схема

С 26 ноября 2026 приоритет — сервер. Схема ниже показывает существующую нативную
консоль. В M5.5 рассматривается замена её браузерным интерфейсом с обращениями к
тому же HTTPS API и установкой только Windows-сервера. Выбор веб-технологии и
browser auth/session ещё впереди; фоновые задания и секреты остаются на сервере.

```text
Orkestr Console (Rust/egui) ── HTTPS /v1 ── Orkestr Server (Windows service)
                                            ├── authentication / RBAC / audit
                                            ├── job scheduler + worker pools
                                            ├── inventory + AD discovery
                                            ├── package execution engine
                                            ├── release discovery + downloader
                                            ├── SQLite / migrations / backup
                                            ├── DPAPI secret store
                                            └── content-addressed artifact store
                                                     │
                                   WinRM / CIM / approved legacy DCOM
                                                     │
                                            Windows endpoints (no agent)
```

Клиенты не выполняют централизованные задания от своего имени и не читают SQLite
на сетевой папке. Все проверки подключения и доступа к пакетам — с сервера.
Local mode допускает co-located backend, но использует тот же application service,
а не отдельную логику. Для фоновой работы нужна служба; UI не заменяет службу.

## Рекомендуемое разбиение будущего Cargo workspace

```text
crates/
  domain/          IDs, entities, version comparison, state machines, validation
  storage/         SQLite repositories, migrations, unit of work
  credentials/     DPAPI, ACL, metadata, zeroizing buffers
  windows/         PowerShell adapter, WinRM, CIM, ADSI, process control
  inventory/       discovery, scans, software normalization, collections
  deployment/      steps, package revisions, target plans, result reconciliation
  library/         JSON/HTML extractors, regex, release pipeline, download rules
  scheduler/       jobs, leases, per-device locks, quotas, maintenance windows
  api/             DTOs, auth, RBAC, conflict checks, request limits
  server/          service lifecycle, composition root, background workers
  console/         egui views, client API, themes, localization
tests/             contract/integration/upgrade fixtures
```

M1 реализован пятью crates: `domain`, `storage`, `application`, `console`, `cli`. Это карта будущих границ,
не требование создать 11 пустых проектов. CLI временно использует co-located
repositories; не открывать его SQLite через SMB. Общая служба/API — M2.
Нативная UI не вызывает repositories: `application` владеет Store на отдельном bounded
worker, команды/ответы ограничены, preview дебаунсится и версионируется token/generation.

## Данные

| Сущность | Основные поля / инварианты |
|---|---|
| Device | UUID, canonical DNS, aliases, AD objectGUID, site_id, source, disabled, connection_profile_id |
| DeviceScan | UUID, device_id, timestamps, provider statuses, raw details, warnings; immutable snapshot |
| InstalledSoftware | device_id, scan_id, product identity, publisher, version_raw/normalized, architecture, scope |
| Collection | UUID, static members либо typed expression; preview и версия |
| CredentialProfile | UUID, label, username, kind, scope, secret_ref, owner_identity, updated_at; нет пароля |
| ConnectionProfile | UUID, transport, auth, port, TLS policy, credential_profile_id; separate discovery/execution scope |
| Application | UUID, display/publisher, channel, supported architectures, detection rule, source configuration revision |
| Release | UUID, app/channel/arch, version_raw/normalized, discovery evidence, final URL, state |
| Artifact | SHA-256, size, protected path, verification result, expected publisher, quarantine state |
| PackageRevision | immutable steps, arguments, detection rules, artifact hashes, author/review state |
| Policy | collection, app/channel, auto flags, approval/pilot, time zone/window, site quotas, priority |
| Job / TargetTask / Attempt | IDs, idempotency key, snapshot target list, package revision, lease, state, result |
| Operator / Token | identity, roles, hashed token, expiry/revoked, creator |
| AuditEvent | actor, verb, target, revision/hash, timestamp, correlation ID, safe result |

Новая SQLite на локальном NTFS. WAL + busy timeout + transaction boundaries,
schema_version и последовательные миграции. История вывода отдельно с размером и retention,
а не в снимке всей конфигурации. Большие артефакты — файлы по hash, не BLOB базы.
Физические индексы и page size выбирать по benchmark реальной нагрузки.

Изменение URL, источника, installer args, detection, signing policy создаёт новую
конфигурационную/пакетную revision и сбрасывает относящееся к ней одобрение.
Утверждённая задача ссылается на immutable revision, не на «последнюю» версию.

## API: предлагаемый контракт, не существующие endpoints

- `POST /v1/jobs` → `202 {job_id}`; `GET /v1/jobs/{id}` → состояние/этапы.
- `POST /v1/jobs/{id}/cancel` → запрос отмены; не ложное «всё остановлено».
- `GET /v1/devices`, `/collections`, `/applications` с pagination/filter.
- `POST /v1/connections/preview` / `test`: asynchronous job, preview не импортирует.
- `POST /v1/credential-profiles`: write-only secret, сервер шифрует сам.
- `GET /v1/credential-profiles`: только metadata; чтения plaintext нет.
- `POST /v1/applications/{id}/check`: получить проверяемый release candidate.
- `POST /v1/releases/{id}/download` / `approve` / `reject`: независимые действия.
- `PUT /v1/policies/{id}` с expected revision / `If-Match`; 409 при конфликте.
- `GET /v1/events?after=cursor`: bounded incremental events; потеря курсора → resync.

Аутентификация/право действия проверяются до чтения большого body и запуска работы.
Idempotency-Key на создание job/approval; actor приходит из auth, не из request body.
Лимиты на размер, время, частоту, параллелизм, длину вывода и размер страниц.
Ошибки типизированы: `TLS_NAME_MISMATCH`, `CREDENTIAL_UNAVAILABLE`, `CONFLICT`,
`DETECTION_UNKNOWN`, `SOURCE_CHANGED`, `SITE_QUOTA_WAIT`; human hint отдельно.

## Персональная аутентификация

Для первой версии предложение: одноразовое локальное server bootstrap,
затем отдельный криптографически случайный token для каждой консоли/оператора,
сервер хранит только hash, срок и отзыв. Token на консоли — local CurrentUser DPAPI.
Роли Admin / Operator / Viewer, Approver при требовании разделения обязанностей.
Windows SSO — возможное расширение, не «автоматически уже есть» из-за домена.

## Транспорт к ПК

Domain: Kerberos/Negotiate с проверяемым именем; prefer HTTPS, HTTP/Kerberos
только как явно утверждённая сетевой политикой совместимость, без Basic/unencrypted.
Workgroup: WinRM HTTPS, доверенная цепочка и совпадение SAN, локальный credential profile.
DCOM и Remote Registry из Inventory — opt-in legacy, не обязательный путь в новой версии.
Connection test выполняет DNS → TLS/transport → auth → read-only CIM → доступ к staging.
Никаких автоматических правок firewall, UAC, TrustedHosts при открытии окна.

## Важное ограничение без агента

Сервер должен видеть каждый ПК через LAN/VPN и иметь разрешённый remoting.
Нельзя гарантировать доступность ноутбука вне сети, интерактивные уведомления,
точную отмену detached installer или автономное исполнение после разрыва связи.
Эти состояния отражать честно, агента не имитировать.
