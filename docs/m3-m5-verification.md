# M3–M5: фактические проверки 26–27 ноября 2026

Серверная реализация доступна через HTTPS API. Это протокол выполненных проверок,
а не заявление о полной production-приёмке. [Контракт](m3-m5-server.md),
[roadmap с отдельными acceptance пунктами](../roadmap.md).

Для публикации имена ПК/домена заменены на `PC01`, `SERVER01`, `example.test`.
Исходные отчёты и приватная версия документа сохранены вне Git в локальном архиве.

## Автоматические проверки

Windows, GNU Rust toolchain, локальный GCC из существующего `tools/w64devkit/bin`.
Изменений системного PATH/rustup default не делалось. Итоговые команды:

```powershell
& .\tools\check-core.ps1 -Gnu -CompilerBin (Join-Path $env:LOCALAPPDATA 'OrkestrEMP/toolchains/w64devkit/bin')
```

Результаты полного прогона сохраняются в игнорируемом `test-results/`:
`m3-m5-final-check.log` (полный прогон), `m3-m5-clippy.log`, `m3-m5-build.log`.
В итоговом workspace — **138 тестов**. Форматирование, Clippy с `-D warnings`,
тесты и сборка прошли; завершение проверок — 27 ноября 2026.
Проверяются прежние функции M1/M2/M3 и новые сценарии:

- JSON Pointer, HTML/attribute/regex/manual, numeric normalization, URL template
  injection, неоднозначность и лимиты selectors; «Полная версия» с изменением ID ссылки.
- Public/private/reserved IPv4/IPv6 и IPv4-mapped IPv6, exact internal CIDR exceptions.
  Реальный локальный HTTPS fixture: strong ETag/206 resume, 200 reset, hash всего файла,
  metadata redirect block, encoded body/size cap, недоверенный CA/неверное имя.
- Payload pacing и прерывание; concurrent SQLite claims не обходят global quota.
  Site/default/strict sequential, midnight/DST и retry jitter.
- Known/approved references и software matching без записи derived полей в scans;
  source revision reset, stale verification после approval, downgrade guard.
  Ограниченный список истории не обрезает эталоны или проверку downgrade.
- Неизменяемые nested workflow revisions, validation путей, checkpoints,
  восстановление неизвестного результата и общая блокировка устройства.
  Неудачный SystemScan внутри workflow сохраняется отдельным report, оставляя
  прежний scan head; checkpoint получает безопасный diagnostic code и report ID.
- RBAC новых API, конфликты revision, idempotency, запрет publish до approval,
  отдельное evidence-разрешение Admin без ложного success.
- Независимые automation flags, manual approval default, signed unattended update
  с неизменным рецептом; rejection другого publisher/name/version/architecture/source,
  unsigned/hash-only candidate и нового hash при той же версии.
- Pilot → soak → production, порог отказов, unknown/source failure/revoked owner/
  policy revision pause; отмена детей и сохранённый retry_at.
- Durable dispatch cursor после reopen проходит backlog из 300 заданий при batch 256.
- Реальный локальный безвредный EXE с большими stdout/stderr, quoting без shell,
  3010 и декорированными WinRM scalar values. Детектор проверен на absent/older/newer/
  unknown/equal numeric formatting, overflow и явно выбранной точности версии.

Эти fixtures не устанавливают vendor ПО и не перезагружают машины. Crash/unknown
проверки моделируют границы в SQLite; они не заменяют физический power-loss test.

## Разрешённый доменный пилот

Владелец разрешил этот ПК и SERVER01. Локальный `PC01` состоит в `example.test`.
27 ноября подключение по полному имени `PC01.example.test` прошло WinRM HTTPS/Kerberos.
Прежний отказ локального подключения не потребовал изменения настроек или обхода TLS.
На `SERVER01.example.test` также доступен WinRM HTTPS/Kerberos с проверкой сертификата.
Использован отдельный профиль **ServerIdentity**, явно выбранный для Execution +
Kerberos. Пароль текущего пользователя не извлекался и не сохранялся.

Сервер запускался foreground в отдельном защищённом каталоге
`test-results/wsus-assistant-pilot`, с localhost HTTPS, собственным доверенным
клиенту CA и DPAPI token. Служба Orkestr SCM не регистрировалась. Политики после
пилота отключены; тестовый сервер остановлен. Правки firewall, WinRM listener,
domain membership и системного TLS trust не выполнялись.

| Сценарий | Наблюдаемый результат |
|---|---|
| WinRM перенос файла 43 390 160 bytes | Подтверждённые offsets, 47 051 ms, заданный cap 4 MiB/s |
| Synthetic EXE exit 0 | Succeeded, INSTALL_VERIFIED |
| Повтор того же installed EXE | Succeeded, ALREADY_INSTALLED, installer пропущен |
| Synthetic EXE exit 3010 | RebootRequired, INSTALL_VERIFIED_REBOOT_REQUIRED; реального reboot нет |
| Повтор synthetic 3010 | ALREADY_INSTALLED, повторного запуска нет |
| LocalArtifact → check → quarantine/hash → manual approve → publish | Все переходы подтверждены, до approval publish отклонён |
| Unattended policy на одном SERVER01, pilot 1 + soak | Succeeded, ROLLOUT_COMPLETE; уже установленный fixture пропущен |
| Workflow Command/PowerShell/Sleep/SystemScan | WORKFLOW_COMPLETE, подтверждены 4 шага; 3010 сохранён без reboot |
| Закрытие API-клиента на время обоих policy/workflow пилотов | Сервер продолжил работу; результат прочитан после нового подключения |
| Параллельная передача по 8 MiB на SERVER01/PC01 | MAX_ACTIVE=2, 16 064 / 16 062 ms, общий payload cap 1 MiB/s |
| Strict sequential на тех же двух целях | MAX_ACTIVE=1, 8 229 / 8 063 ms, общий payload cap 1 MiB/s |

Synthetic fixture — `tools/pilot-fixture.cs`, создаёт только отдельные markers/
launches.log в `C:\OrkestrPilot\Fixture0` и `Fixture3010` и скрытые uninstall records
`{5D6444C6-3D63-4E1F-AD2D-1D5000050000}` / `{5D6444C6-3D63-4E1F-AD2D-1D5000053010}`.
Не создаёт служб, сетевых соединений или запроса reboot. Это проверка механизма,
а не успешная установка Ассистента или другого vendor продукта.

Оставлены опциональные явно запускаемые примеры `catalog_policy_pilot`,
`workflow_readonly_pilot`, `library_source_pilot`, `two_device_transfer_pilot`.
Три последних не устанавливают ПО. Two-device pilot создаёт уникальные защищённые
каталоги с marker, копирует synthetic payload и удаляет только два проверенных
файла и пустой каталог без рекурсивной очистки; неизвестные каталоги не затрагиваются.
Исследовательские vendor pilot helpers с решениями по конкретным логам перенесены
в игнорируемый `test-results/pilot-tools/`: использовать их как общий рецепт нельзя.

Примеры отчётов в test-data: `synthetic-0-result.json`, `synthetic-3010-result.json`,
`catalog-policy-pilot-result.json`, `readonly-workflow-pilot-result.json`.
Они могут содержать hostname/actor IDs и не предназначены для публичной публикации.

В первом двухкомпьютерном прогоне оба transfer прошли в обоих режимах и квоты не
превышались. Но в sequential фазе SystemScan на PC01 завершился Failed после
подтверждённой очистки payload. Первый прогон целиком не засчитан; отчёты сохранены
как `two-device-transfer-first-attempt.json/.log`. При отдельном read-only повторе
адаптер вернул все 23 providers, включая предусмотренные Partial. Причина исходного
отказа не установлена. Исправлен обнаруженный недостаток workflow: Failed scan
теперь тоже сохраняется вместе с диагностикой, а не теряется из истории Inventory.

Повторный двухкомпьютерный прогон прошёл полностью в обоих режимах: 4 workflow по
5 подтверждённых шагов, verified transfers и сохранённые Partial Inventory reports.
Проверены actual lease peaks 2/1 и минимальные времена pacing для 8 MiB при
512 KiB/s на цель в parallel и 1 MiB/s в sequential. Это проверка payload cap,
не ограничителя wire Mbps. Исходные scheduler settings восстановлены; leases и
device locks после завершения отсутствуют, auto-check/download/deploy отключены.
Read-only проверка обеих машин подтвердила отсутствие всех четырёх уникальных
пилотных каталогов из обоих прогонов. Отчёты: `two-device-transfer-pilot-result.json`,
`test-results/two-device-transfer-pilot.log`, `test-results/two-device-cleanup-verification.json`.

## Ассистент: загрузка прошла, quiet install отклонён

Согласованный [официальный источник](https://мойассистент.рф/скачать) показал 7.1
от 01.10.2026. Из блока с точным заголовком «Полная версия» получена ссылка на
Download/1638. В рабочем конфиге нет привязки к этому числу:
[examples/library/assistant.json](../examples/library/assistant.json).

Полный цикл **через API/worker сервера**, без approve/publish/install:
Check Succeeded → Download Succeeded → Release Verified. Полный EXE —
43 390 160 bytes, SHA-256
`268b07a9c4784c4996bface752f63d89d094ec6094bc7ca83987b0238ec1ec03`.
Authenticode Valid, subject:
`E=admin@safib.ru, CN=SAFIB LLC, O=SAFIB LLC, L=Voronezh, S=Voronezh Oblast, C=RU`.
Сервер прочитал PE metadata: «Ассистент», publisher «САФИБ», x86,
ProductVersion 7.1.2609.301. Отчёт: `source-download-pilot-result.json` и
`test-results/assistant-library-pipeline.log`. Извлечение MSI properties также
проверено без запуска; у вложенного vendor MSI architecture отсутствует в Template,
поэтому automatic metadata-based promotion для такого MSI запрещается.

При исследовании тихой установки на SERVER01 обнаружен вложенный Advanced Installer
bootstrapper. Пробные параметры либо ожидали взаимодействия, либо завершали внешний
MSI без установки приложения. Позднейший запуск вложенного setup выполнил custom
action с `ASTTEMPDIR=C:\` и начал ошибочное копирование содержимого корня в каталог
приложения; vendor log подтвердил ошибку. Дальнейшие vendor установки остановлены.
Неизвестные исходы не превращены в успех: сохранена история, после проверки отсутствия
процесса/службы они вручную закрыты как Failed с evidence.

Пилот также выявил реальную ошибку Orkestr: remoting декорирует строки дополнительными
свойствами, и JSON detection становился объектом. Исправлены явные scalar casts
и отсутствие служебного вывода async drain; добавлена регрессионная проверка.

### Оставшееся состояние SERVER01

Последняя read-only проверка: процессов Assistant/ast_service/msiexec нет,
AstService отсутствует. **Ассистент установленным не считается.**
Остался новый `C:\Program Files (x86)\Ассистент`, созданный
2026-11-26 13:32:13.999 UTC (16:32 Москва). На верхнем уровне — `$Recycle.Bin`
и `Documents and Settings`, скопированные неудачной custom action. Не путать этот
каталог с исходными каталогами в `C:\`; не переходить по junctions при очистке.
В `C:\OrkestrPilot` также остаются test payloads/markers/logs; для неизвестных
исходов могут сохраняться отдельные временные staging directories.

Попытка ограниченной очистки только нового каталога была отклонена автоматической
проверкой: **blocked by policy**. Обход другим shell/скриптом не выполнялся.
Пользователю отправлен отдельный запрос на удаление этого точного каталога;
ответ пока не получен. Удаление и проверка его результата не заявляются выполненными.
Нельзя удалять сертификаты/общие зависимости vendor вслепую: их исходное состояние
не зафиксировано. Широкая автоматическая очистка сервера в этот инкремент не входит.

## Что остаётся для acceptance

- Реальный рабочий vendor рецепт MSI/EXE с install/upgrade/downgrade в домене и
  рабочей группе; Ассистент требует документированного или согласованного рецепта.
- Workgroup pilot с отдельными credentials;
  SCM/service identity, restart/reboot/power-loss и production PKI lifecycle.
- Несколько согласованных реальных release sources; корпоративный HTTPS proxy,
  disk-full и реальная сетевая потеря во время resume.
- Нагрузочная приёмка целевого парка/нескольких площадок. Два доменных ПК прошли
  payload/quotas pilot; это не проверка 50 ПК, насыщения канала или wire Mbps cap.
- Unattended workflow с произвольными scripts не включён: auto approval
  ограничен library-backed managed installers с проверяемым неизменным рецептом.
  Смена MSI ProductCode также требует ручной ревизии рецепта/политики.
- Browser UI/дизайн и browser auth относятся к M5.5; retention, крупная история,
  release packaging, полный security audit и нагрузочные SLO — к M6.

Результат: серверные функции по M5 реализованы и проверены локально, на SERVER01 и PC01.
Открытые пункты приёмки сохранены в roadmap, успешная установка Ассистента и
полная готовность к production не заявляются.
