# Roadmap Orkestr EMP

Порядок основан на зависимостях и рисках. Календарные сроки — после согласования
масштаба и состава команды; это не обещание «несколько дней».

**Приоритет владельца, 2026-11-26:** сначала довести сервер по M3–M5.
Затем пересмотреть дизайн и обращения интерфейса к API; предпочтительный вариант
для оценки — работа через браузер без отдельного Rust-клиента, с установкой только
Windows-сервера. Окончательная схема поставки выбирается на этапе M5.5.
Текущая egui-консоль остаётся средством проверки сервера; её дизайн не считается итоговым.

## M0 — Исследование и проектирование

- [x] Прочитать ключевые исходники, модели, transport, DPAPI, storage и отчёты.
- [x] Уточнить топологию, агентов, автоматику и хранение паролей.
- [x] Два интерактивных offline HTML-направления + черновой help.
- [x] Аудит, требования, архитектурные предложения, миграция, threat model, тест-план.
- [x] Выбран макет A, сохранить знак Orkestr; аудитория — небольшие/средние организации,
  начальный сценарий владельца около 50 ПК, open-source направление.
- [x] Уточнены static/dynamic collections, preview состава/ПО, library-relative versions и RAM.
- [x] Макет A дополнен группами/подгруппами, редактором, переносом и учебным пересчётом.
- [ ] По мере этапов уточнить сетевые условия, ОС, роли, первые приложения и лицензию продукта.

**Выход:** владелец одобрил scope и critical decisions; unresolved вопросы видны.

## M1 — Единое ядро без массового исполнения

- [x] Новый Cargo workspace в этой папке; ограниченная адаптация оригинального MIT-кода с attribution.
- [x] DeviceId и общий локальный реестр, коллекции, immutable PackageRevision, schema migration v1.
- [x] Static/dynamic collection engine, редактор И/ИЛИ, numeric >=/<=, дерево со счётчиками,
  preview количества/списка ПК/ПО; RAM >=16, >=8, <8 и 8–<16, Unknown отдельно.
- [x] Вложенные папки/выборки, parent ID, перенос без циклов, union без дубликатов.
- [x] Общая локальная egui-консоль A, темы, карточки, навигация; UI отделён от application service.
- [x] Безопасный read-only импорт копий JSON/SQLite, dry-run и rollback report.

**Выход:** в одной консоли видны импортированные ПК/пакеты; ничего не запускается само.

**M1 реализован, 2026-11-06:** `domain` + `storage` + bounded `application` + нативная
`console` A + `cli`. SQLite v2, immutable scans/revisions, typed И/ИЛИ/RAM/Unknown,
дерево/union/перенос, preview с generation/tokens, подробные сохранённые данные ПК.
Импорт выбранных копий сохраняет legacy-автоматику только в выключенном review;
неподдержанные старые правила не активируются с новым смыслом. Installer files,
credentials/config и history outputs не переносятся и не исполняются.

Проверки на синтетических копиях реальных форматов, не production migration sign-off.
Живая библиотека — M4, служба/API/совместная работа — M2; M1 не объявляет их готовыми.
Команды: [docs/m1-console.md](docs/m1-console.md),
результаты и Not run: [docs/m1-console-verification.md](docs/m1-console-verification.md).

## M2 — Сервер, права и учётные данные

- [x] SCM entrypoint/installer helper, защищённые paths/ACL, startup health (код; SCM-приёмка ниже).
- [x] HTTPS API, персональная авторизация/отзыв, Admin/Operator/Viewer и аудит.
- [x] Async read-only jobs API, bounded workers/events, конфликты редактирования.
- [x] Separate AD/execution profiles, server-side DPAPI CurrentUser.
- [x] Окна подключений/credentials, фиксированные read-only AD/CIM adapters (domain test не проведён).
- [ ] Приёмка SCM/reboot под постоянной account, две физические машины/другая identity,
  положительные и отрицательные AD/WinRM проверки в согласованном домене; PKI/revocation review.

**Выход:** две консоли, один сервер; secrets переживают restart под той же identity;
другая identity даёт явную блокирующую ошибку. Viewer не может выполнить команду.

**M2 dev-контур, 2026-11-06:** `protocol` + `credentials` + `client` + `server`,
SQLite v3, remote-режим существующей egui-консоли. Локальные HTTPS/DPAPI проверки
не заменяют Windows/domain пилот; **M2 acceptance пока не закрыта**.
24 ноября с доменного ПК подтверждены LDAPS RootDSE и WinRM HTTPS/CIM на согласованных целях
под текущей учётной записью. Проверки через профили Orkestr и SCM/reboot остаются открытыми.
Инструкция: [docs/m2-server.md](docs/m2-server.md), результаты/Not run:
[docs/m2-verification.md](docs/m2-verification.md). Deploy/инвентаризация по сети остаются M3.

## M3 — Инвентаризация и управляемый Deploy

- [x] M3.1 offline-ядро: ручной/CSV preview + атомарный apply, typed F21 provider reports,
  software normalization и Unknown/Partial, immutable SQLite reports/scans (без сети/UI wiring).
- [x] M3.2: manual/CSV окно и API preview/apply с Admin/RBAC/backup/durable receipt;
  on-demand безопасные статусы providers/последней попытки в карточке (local + HTTPS).
- [x] M3.4. Предпросмотр OU через LDAPS, ручной ввод и CSV. Сетевой CIM-скан
  с нормализацией ПО, статусами Partial/Unknown и 23 разделами F21 в карточке.
- [x] M3.3: сохранённые профили WinRM HTTPS с выбором Negotiate для домена/рабочей группы
  или Kerberos для домена. Приёмка с сохранёнными учётными данными остаётся открытой.
- [x] M3.5. Управляемый MSI на одном ПК, неизменяемая ревизия и SHA-256,
  одобрение, блокировка ПК, история попыток, detection и статус 3010.
- [x] Перезапуск и сверка результата MSI, отмена до запуска, до трёх попыток
  с повторным одобрением после отказов до установки.
- [x] M3.6. Серверная установка EXE: отдельный staging, явная тихая командная строка,
  immutable revision, одобрение, SHA-256 и проверка системной GUID-записи/версии.
  Реальные EXE-установки в домене/рабочей группе ещё не приняты.
- [x] Серверное исполнение всех штатных шагов Deploy: immutable workflow,
  checkpoints, NestedPackage, общий device lock и ручное разрешение неизвестного исхода.
- [ ] Полная приёмка M3 в домене и рабочей группе; реальная тихая установка Ассистента.

**Выход:** разрешённый domain/workgroup пилот проходит; после разрыва связи нет
ложного success и слепого повторного installer.

**M3.1, 2026-11-06:** по выбору владельца начато offline-ядро M3 без подключений/
установок. `inventory` + SQLite v4 + local CLI `discover`; действующие M1/M2 сохранены.
Это частичная реализация первого пункта, не сетевой Inventory и не готовый Deploy.
M2 acceptance по-прежнему открыта. Контракт/команды: [docs/m3-inventory-core.md](docs/m3-inventory-core.md),
проверки/Not run: [docs/m3-inventory-verification.md](docs/m3-inventory-verification.md).

**M3.2, 2026-11-06:** новое окно использует существующий bounded Command/Event transport;
клиентский файл читается на клиенте, сервер получает текст, не path. SQLite v5 receipt
обеспечивает replay успешного apply после restart. Viewer читает только safe provider metadata.
AD/CIM adapters/deploy и M2 acceptance не закрыты. Контракт/эксплуатационные ограничения:
[docs/m3-console-api.md](docs/m3-console-api.md), проверки: [docs/m3-console-api-verification.md](docs/m3-console-api-verification.md).

**M3.3, 2026-11-24.** Режим аутентификации WinRM сохраняется в серверном профиле.
SQLite v6 защищает его от запуска старым сервером. Проверка читает название ОС через CIM.
Сборка, Clippy и 99 тестов прошли. Оба режима проверены с имитацией CIM-сессии,
Kerberos HTTPS проверен на согласованном доменном сервере под текущей учётной записью.
Порядок настройки и границы проверки описаны в [docs/m2-server.md](docs/m2-server.md).

**M3.4–M3.5, 2026-11-24.** Очередь выполняет запрос OU и сканы через сохранённые
профили. Новая ревизия MSI регистрируется отдельно от импортированных определений.
После одобрения сервер проверяет хеш локально и на цели, запускает Windows Installer
и сверяет версию. Если исход установки неизвестен, блокировка ПК сохраняется до сверки.
SQLite v7 хранит планы и попытки. [Порядок пилота](docs/testing.md#ручной-пилот-m3).
Форматирование, Clippy, сборка и 103 теста прошли. Проверка реальной установки остаётся
в пилоте на тестовой ВМ.

**M3.6, 2026-11-26.** EXE использует существующую очередь, блокировку ПК и сверку
неизвестного исхода; параметры закреплены в ревизии и запускаются без shell.
SQLite v8 с backup v7; старые MSI сохраняют прежнее поведение. Регистрация EXE пока
через HTTPS API, просмотр/одобрение доступны в текущей консоли.
Контракт и границы: [docs/m3-managed-installers.md](docs/m3-managed-installers.md).
Форматирование, Clippy, 113 тестов и сборка прошли; локальный тестовый EXE ничего
не устанавливает. Приёмка реального WinRM/EXE остаётся в пилоте.

## M4 — Настраиваемая библиотека

- [x] JSON API / HTML CSS selector / text regex / manual sources.
- [x] URL templates и link extraction, test preview с evidence и ambiguity checks.
- [x] Version/channel/architecture normalization, installed-software matching.
- [x] Коллекции по связанной записи библиотеки: installed version =/< latest known,
  отдельный эталон approved; пересчёт состава при смене версии без автозапуска установки.
- [x] Downloader, cache by hash, quarantine, Authenticode/hash policy.
- [x] SSRF/redirect/DNS guards, explicit trusted proxy, bounded requests, .part/resume.
- [ ] Полевая приёмка нескольких реальных источников и корпоративного proxy.

**Выход:** несколько согласованных реальных источников проверены, broken selector
блокирует rollout; download никак не означает install.

## M5 — Политики и сеть

- [x] Независимые auto-check / auto-download / auto-deploy flags.
- [x] Manual approval default, unattended opt-in + pilot/soak/error threshold.
- [x] Global/site/phase quotas, strict sequential mode, jitter, persisted retry.
- [x] Time windows/time zones, preview waves, fair scheduling, change history.
- [x] Измеряемый transport с ограничением payload bytes/s и WinRM block acknowledgements.
  Wire Mbps limiter не заявляется.
- [x] Конкурентный transport/quotas пилот на PC01 и SERVER01: по 8 MiB,
  global cap 2, strict sequential cap 1, проверка хешей и очистка test payloads.
- [ ] Нагрузочная приёмка целевого парка/нескольких площадок и SCM/reboot lifecycle.

**Выход:** конкурирующие jobs/retries не обходят квоты; трафик измерен в тестовой сети;
закрытие всех консолей не влияет на выполнение политик службой.

**M3–M5, 2026-11-26:** серверная реализация перечисленных функций готова через API,
SQLite v10 с backup; новую UI не добавляли. На SERVER01 прошли synthetic EXE 0/3010,
повтор с ALREADY_INSTALLED, библиотека → approval → policy/pilot/soak и workflow
Command/PowerShell/Sleep/SystemScan при закрывавшемся клиенте. Ассистент: официальный
HTML source → download → Authenticode → Verified прошёл, quiet install не принят.
Исследовательский установщик оставил каталог на SERVER01; очистка ожидает решения после
автоматического запрета удаления. Это не закрывает оставшиеся acceptance пункты.
27 ноября прошёл отдельный двухкомпьютерный payload/quotas пилот. Первый отказ
SystemScan на PC01 не засчитан как успех; сохранение Failed report и диагностики
в workflow исправлено, повторный полный прогон прошёл. Пилотные каталоги удалены,
исходные квоты восстановлены; каталог Ассистента к этой очистке не относится.
[Контракт/API](docs/m3-m5-server.md), [результаты/ограничения](docs/m3-m5-verification.md).

## M5.5 — Дизайн, веб-интерфейс и схема поставки

- [ ] Разобрать замечания к дизайну и ошибки по результатам пользовательской проверки.
- [ ] Оценить веб-сервис без отдельного Rust-клиента: браузер → HTTPS API → Windows-служба;
  установка и обновление только серверной части. Зафиксировать решение и ограничения.
- [ ] Переработать веб-дизайн и основные сценарии: ПК, группы, пакеты, задания,
  подключения, ошибки и одобрение; сохранить знак Orkestr.
- [ ] Подключить выбранный интерфейс к готовому API, определить browser auth/session,
  обновление статусов, конфликты и восстановление связи. Задания продолжаются без браузера.
- [ ] Проверить интерфейс и комплект поставки перед выпуском; пересмотреть необходимость
  нативной консоли в дистрибутиве.

**Выход:** согласованы дизайн и способ доступа; при выборе веб-варианта пользователь
работает через браузер, устанавливается только сервер. Нативный клиент не обязателен.

## M6 — Приёмка и выпуск

- [ ] Подробная версия offline help с реальными screenshots/командами готового build.
- [ ] SCM install/start/stop/reboot, backup/restore, upgrade/uninstall и recovery drill.
- [ ] DPI/keyboard/contrast/localization, logs retention и privacy review.
- [ ] Reproducible release, signatures, license manifest, dependency/security review.
- [ ] Производственный пилот и опубликованные известные ограничения.

**Выход:** выполнены ворота `docs/testing.md`, владелец разрешил ограниченный rollout.

## Позже, отдельно от V1

Агент и enrollment/mTLS, branch caches/relay, визуальный live element picker,
Windows SSO, расширенный RBAC/four-eyes, LAPS/gMSA integrations, отчёты.
Веб-интерфейс перенесён на рассмотрение перед выпуском в M5.5 по запросу владельца.
Остальные пункты не включать скрыто в первую версию.
