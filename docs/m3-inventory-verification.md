# Проверки M3.1 — offline inventory core

Отчёт инкремента M3.1; текущий прогон — [M3.2](m3-console-api-verification.md).

Дата: 2026-11-06. Windows, GNU Rust toolchain, `x86_64-pc-windows-gnu`.
Только собственные синтетические fixtures/temp DB; реальные данные владельца не импортировались.

## Фактически выполнено

Из корня `Orkestr EMP`:

```powershell
$env:TMP='C:\Temp\opencode'
$env:TEMP='C:\Temp\opencode'
.\tools\check-core.ps1 -Gnu -CompilerBin 'C:\Users\it\Documents\Craft Anything\pdqinventoryconsole-2\app\tools\w64devkit\bin'
```

| Проверка | Результат |
|---|---|
| `cargo ... fmt --all -- --check` | PASS |
| `cargo ... clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo ... test --workspace --locked` | **81 passed, 0 failed** |
| `cargo ... build --workspace --locked` | PASS, debug console/server/CLI |
| `py -3 tools/validate.py` | PASS: 4 HTML, local links/anchors, JSON/CSS/offline guard |
| `.\target\debug\orkestr-cli.exe --help` | PASS, команда `discover` доступна |

Разбивка: domain 12; inventory 6 discovery + 9 scans; storage 6 core + 9 import +
4 upgrade/review + 7 inventory; application 5; console 9; CLI 3; client 2;
credentials 1; server API 6 + Windows runtime 2. Пустые unit/doc harnesses не включены.
По сравнению с M2 — 24 новых теста; API privacy test расширен для M3 evidence.

### Discovery / CLI

- Manual: canonical case/trailing dot, CRLF/BOM, blank lines, exact duplicate rows;
  DNS не вызывается. IP/URL/injection/path/wildcard/invalid host отклоняются.
- CSV: quotes/escaped quotes/Unicode/BOM, optional headers, сохранение name/site при
  пустых полях; неизвестные/duplicate headers, credentials columns, uneven rows,
  malformed quoting/controls/empty/oversized input и >1000 records отклоняются.
- Preview не пишет в БД. Add/update/no-op/duplicates/conflicts; конфликтующие строки,
  разные alias одного UUID и ambiguous owner блокируют apply. Short/FQDN не сливаются.
- Обновление по alias сохраняет current host/UUID/AD GUID/disabled/OU/domain/private
  metadata. Stale preview и preview другой БД с такой же generation отклоняются.
- CLI **реальным subprocess**: без базы нет implicit create; dry-run → пустой реестр;
  apply → новый backup → одно устройство без scan/job; источник не изменён;
  повторный backup не перезаписывается; parse errors не отражают synthetic secret.

### Provider reports / нормализация

- 23 F21 providers; missing → NotCollected, общий Unknown/Failed не создаёт scan.
  Независимый failure не скрывает удачные raw sections и не становится пустым успехом.
- Full fixture Complete требует все providers и непустую RAM evidence. Противоречивые
  states, duplicate providers, reversed times, non-object/oversized rows/report — reject.
- RAM: sum integer/string Capacity; partial/zero/missing/noninteger/overflow → Unknown.
  Округлённый MemoryGB не используется. Timestamp снимка — начало сбора.
- ПО: raw/normalized numeric version, architecture aliases, product UUID, registry
  identity, machine/user SID scope; точные дубликаты убраны, разные publisher/scope/
  version не объединены. Unknown version/arch не угадывается; library mapping не выполнен.
- HKLM-only и некорректные software rows → Partial. Коллекции сохраняют Unknown;
  пустой Publisher при фильтре по издателю — Unknown, известная другая identity — No.

### SQLite / регрессии

- Report + scan атомарны и переживают reopen. Duplicate report UUID/неизвестный device
  откатывают scan/software rows/head и generation. No-op discovery не увеличивает generation.
- Failed/Unknown runs сохранены отдельно; прежний scan UUID/date не обновлены.
  Out-of-order reports не вытесняют newer head; latest report упорядочен по числовым
  seconds/nanos и доступен отдельным indexed lookup.
- v1/v2/v3 → v4 с consistent non-overwriting backup. v3 fixture сохраняет generation,
  synthetic ciphertext/canary byte-for-byte в текущей БД и читаемом pre-upgrade backup.
  Writable reopen v4 не создаёт повторный backup. Старые import/rollback/review tests проходят.
- M2 API: Viewer **и Operator** не получают private metadata/M3 scan.details/evidence;
  Admin видит raw fixture, нормализованный software summary остаётся видимым Viewer.
  Viewer preview также очищен. Остальные RBAC/expiry/revoke/conflict/bounded jobs tests PASS.
- Реальные **localhost-only** HTTPS, personal clients, DPAPI CurrentUser и перезапуск
  собственного test-server child повторно PASS. Это не AD/CIM scan и не SCM/reboot приёмка.
- Существующие headless egui theme/zoom/pointer/keyboard tests повторно PASS;
  новых окон M3 в этом инкременте нет. HTML-макеты не изменены.

## Not run / не реализовано

- ADSI/OU/page>1000/rename discovery, live CIM/F21 scan, domain/workgroup WinRM,
  настоящие auth/TLS/DNS failures, проверка полноты user-software coverage.
- Новые scan API/jobs/UI/provider-status/latest-failure indication, paging/retention,
  server load/large-volume sign-off. Текущий M2 snapshot limit не заменён пагинацией.
- Deployment, installs, detection, reboot/approval/locks/reconciliation/cancellation/retry.
- M2 SCM/reboot/two physical machines/другая Windows identity/PKI revocation acceptance
  остаются открытыми; лицензия/release/signature/dependency audit также не закрыты.

Ни исходные соседние приложения, ни production DB/secrets, ни службы/firewall/domain
не изменялись. В runtime tests ACL менялись только у собственных новых temp-каталогов.
Нет remote AD/endpoint probes и установок. Успех этих локальных тестов **не закрывает M2/M3**.
Контракт/ограничения/команды: [m3-inventory-core.md](m3-inventory-core.md).
