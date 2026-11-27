# Проверки M3.2 — console/API discovery и safe provider status

Дата: 2026-11-06. Windows, `stable-x86_64-pc-windows-gnu`.
Все registry/report/secret inputs синтетические, DB и protected server directories — собственные temp.

## Выполненный полный прогон

Из корня `Orkestr EMP`:

```powershell
$env:TMP='C:\Temp\opencode'
$env:TEMP='C:\Temp\opencode'
.\tools\check-core.ps1 -Gnu -CompilerBin 'C:\Users\it\Documents\Craft Anything\pdqinventoryconsole-2\app\tools\w64devkit\bin'
py -3 tools/validate.py
```

| Проверка | Фактический результат |
|---|---|
| `cargo ... fmt --all -- --check` | PASS |
| `cargo ... clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo ... test --workspace --locked` | **96 passed, 0 failed** |
| `cargo ... build --workspace --locked` | PASS: debug console/server/CLI |
| `py -3 tools/validate.py` | PASS: 4 HTML, links/anchors, JSON/CSS/offline guard |

Разбивка: application 7; CLI 3; client 2; console 12; credentials 1; domain 12;
inventory 6 discovery + 9 scan; server 9 API + 2 Windows runtime; storage 1 session unit +
6 core + 6 discovery sessions + 9 import + 7 inventory + 4 upgrade/review.
Пустые unit/doc harnesses не включены. Добавлено 15 tests к 81 M3.1;
existing Windows runtime tests расширены, а не посчитаны повторно как новые.

## Проверенные новые сценарии

### Storage / application

- Preview не меняет registry. Actor isolation/confirmation, generation conflict,
  draft replacement (включая invalid input), discard и restart без successful receipt
  блокируют старый apply. Capacity 8 и TTL проверены на собственном session fixture.
- Успешный apply сохраняет devices/receipt/audit одной транзакцией и создаёт согласованный
  backup. Replay тем же owner после reopen даёт тот же receipt, без нового write/backup/audit.
  No-op не меняет generation и не делает backup. Другой owner не читает receipt через apply.
- Существующий backup marker-файл не перезаписан. При synthetic SQLite trigger failure
  на receipt insert откатываются devices/generation/audit, private SQL error не отражается
  пользователю. Удаление только этого собственного test-trigger позволяет ручной retry.
- v1/v2/v3 → v5 старые tests PASS; v4 fixture с Device/report → v5 сохраняет generation/report
  и оставляет читаемый consistent v4 backup. Synthetic security blobs/canary не изменены.
- Safe status отличает latest Failed от прежнего Partial head, не содержит private raw marker;
  no-report не становится Complete. Неизвестный device не создаётся по read-запросу.
- Bounded local application worker: file → text → preview → apply → receipt replay → refresh →
  status. Reader rejects UTF-16/invalid UTF-8, oversized/non-regular/UNC/URL до чтения содержимого.

### API / роли

- Viewer **и Operator** получают 403 на discovery endpoints **до чтения oversized body**.
  Admin input path/backup вместо установленного DTO → 400; text >24 KiB → 422.
- Actor-bound preview, explicit confirmed, unknown fields, duplicate CSV rows и показ
  proposed name; успешный apply/replay; общий registry виден другому персональному actor.
  Stale preview → 409 без нового Device; connection jobs не создаются.
- Inventory status доступен Admin/Operator/Viewer: latest/head IDs/states/counts/codes,
  без raw Environment/SID/evidence; неизвестный DeviceId → 404.
- Сохранены старые permission/body/auth/expiry/revoke/jobs limits и legacy/M3 redaction tests.
  Новые raw report read/write, command/install endpoints отсутствуют.

### Настоящие localhost HTTPS и test-server process

- Новый flow выполнен через **тот же bounded client Command/Event transport, что у консоли**:
  чтение synthetic CSV клиентским worker → HTTPS prepare/apply/replay → refresh второй
  Operator-консоли → safe status; попытка Operator prepare → FORBIDDEN.
- Отдельный реальный `orkestr-server.exe`: prepare/apply → завершение **только собственного
  child process** → новый process → replay исходного preview ID возвращает прежний receipt,
  registry содержит один ПК. Это симуляция потерянного ответа, не внесение packet loss в сеть.
- Все прежние localhost SAN/CA trust negatives, personal clients, DPAPI CurrentUser,
  identity mismatch, restart и single-instance tests повторно PASS.

### UI / регрессии

- Headless egui с реальным pointer/key input: открыть новое окно, Ctrl+N не открывает
  второй group editor, переключить CSV, ввести текст, получить preview, убедиться в отсутствии
  записи, disabled apply без checkbox, явное подтверждение/apply, backup и итоговый Device.
- Старые discovery response tokens игнорируются. Transport error сохраняет исходный
  apply ID для retry, явный CONFLICT сбрасывает preview/подтверждение.
- Status с чужой generation не принимается за данные текущего workspace: карточка
  требует refresh, не смешивая новый head и прежние сохранённые значения.
- Окно input/preview и provider failure states отрисованы light/dark, zoom 100/125/150/200%.
  Existing F21 tabs/group editor/import/review/keyboard/contrast tests проходят.
  Drafts не добавлены в Preferences; HTML A/B и знак Orkestr не перерабатывались.

## Not run / не принято

- Настоящий domain/workgroup AD/CIM/WinRM scan, live provider coverage/user-software scopes;
  ADSI/OU/paging>1000/rename и deployment/install/reboot. Адаптеры не написаны этим этапом.
- Две физические Windows-машины/другая Windows identity, SCM/service/reboot и PKI/revocation:
  незакрытая M2 acceptance. Localhost/owned child tests не заменяют этот пилот.
- Native screenshot/OS file-picker interaction, screen reader/High Contrast, физическая DPI
  матрица и пользовательская accessibility-приёмка. Headless render не равен visual sign-off.
- Fault injection настоящего packet loss/disk-full/backup ACL failure, полная mapped-drive/
  reparse-tree policy, production restore/recovery с новым actor/lost UI preview ID.
- Large registry/backup latency/cancellation/SLO, report/receipt/backup retention, release/
  dependency-license/security/signing audit. API apply backup не принят как длительная job queue.

Ни production данные, ни соседние приложения, ни службы/firewall/domain/TrustedHosts
не изменялись. Нет AD/endpoint probes и установок; ACL затронуты только у собственных
новых test directories, как в M2 runtime tests. **M2/M3 acceptance не закрыты.**
Контракт/ограничения/порядок использования: [m3-console-api.md](m3-console-api.md).
