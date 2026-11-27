# Проверки M2 dev-контура

Дата: 2026-11-06. Windows, Rust 1.99.1, `x86_64-pc-windows-gnu`.
Только локальные синтетические fixtures; не production readiness и не domain sign-off.

## Фактически выполнено

```powershell
$env:TMP='C:\Temp\opencode'
$env:TEMP='C:\Temp\opencode'
.\tools\check-core.ps1 -Gnu -CompilerBin 'C:\Users\it\Documents\Craft Anything\pdqinventoryconsole-2\app\tools\w64devkit\bin'
```

| Проверка | Результат |
|---|---|
| `cargo ... fmt --all -- --check` | PASS |
| `cargo ... clippy --workspace --all-targets --locked -- -D warnings` | PASS |
| `cargo ... test --workspace --locked` | **57 passed, 0 failed** |
| `cargo ... build --workspace --locked` | PASS, debug server/console/CLI |

Разбивка: domain 11; storage 6 core + 9 import + 4 upgrade/review;
application 5; console 9; CLI 2; client 2; credentials 1; server API 6 + Windows runtime 2.
Пустые unit/doc harnesses не включены в число 57.

### Контракт/API (без удалённых ПК)

- Viewer отказан **до чтения oversized body** на jobs/credentials/connections/operators/
  collections. Неизвестного command endpoint нет. Некорректный token → 401.
- Общая группа видна другому actor; stale expected revision → 409.
- Отзыв одного token не отзывает остальных; expired token → 401.
- Ранее queued job отозванного actor блокируется, не доходит до PowerShell probe.
- Очередь 8, overflow → BUSY, повтор Idempotency-Key возвращает прежний job даже при
  заполненной очереди. Queued cancel не меняется в Interrupted при restart;
  остальные незавершённые jobs → Interrupted, не retry.
- Exact-host/kind scopes и TLS-only порты, body limit, safe metadata без ciphertext,
  bounded events/resync. Viewer/Operator не читают private legacy payloads.
- v1/v2→v3 additive upgrade сохраняет generation и оставляет читаемый consistent
  pre-upgrade backup. Старые M1 tests импорта/движка/CLI/UI сохранены.

### Настоящий localhost HTTPS и Windows DPAPI

`crates/server/tests/windows_runtime.rs` создаёт собственные новые protected каталоги
в test temp, сертификаты, пустую БД и **исключительно синтетические** секреты.

- Реальные TLS listener и reqwest clients. Без доверенного cert — ошибка; IP вместо
  DNS SAN `localhost` — ошибка. С явно выбранным cert + верным именем — успех.
- Два bounded `Command/Event` транспорта **того же типа, что использует console**, с
  разными персональными Admin/Operator tokens: load, save, refresh, общий workspace,
  stale save → CONFLICT. Это transport integration, не две физические Windows-машины
  и не два проверенных нативных GUI процесса.
- DPAPI CurrentUser сохраняет secret, API возвращает только metadata; аудит не
  содержит fixture plaintext. Подменённый persisted `owner_sid` → явная блокировка,
  старые blobs и original owner canary не сбрасываются.
- **Отдельный реальный `orkestr-server.exe`**: запуск → secret write → принудительное
  завершение только собственного test child → второй запуск → health/metadata/
  расшифровка исходного secret. Тот же Windows SID/profile; восстановление SQLite WAL.
- Попытка открыть второй runtime при работающем server блокируется.
- Отдельно: CurrentUser DPAPI round-trip, wrong owner и damaged blob; локальный
  DPAPI token file не перезаписывается, HTTP/userinfo/query/path вместо HTTPS origin
  отвергаются клиентом.

### UI / scripts

- Headless egui render четырёх M2 windows (server, credentials, connections,
  operators/audit): light/dark, zoom 100/125/150/200%. Без сети и реальных паролей.
- Viewer can-write/admin guards; persisted Preferences не содержат token/password.
  Старые M1 pointer/keyboard/contrast tests также выполнены.
- PowerShell parser: `tools/install-m2-service.ps1`, `crates/server/src/probe.ps1` —
  syntax PASS, **не исполнение** install helper и не AD/CIM connection test.
- `py -3 tools/validate.py` — PASS: 4 offline HTML, локальные Markdown/HTML links,
  anchors, JSON, CSS tokens, offline JS guard. HTML-макеты не изменялись.

## Что оставалось открытым 6 ноября

- Установка/старт/стоп/reboot SCM под отдельной постоянной account, service logon/profile,
  Windows Event Log/recovery. Helper написан, **на машине владельца не запускался**.
- Другая настоящая Windows identity/другая машина. Подмена metadata SID — negative
  fixture, не доказательство результата полноценного multi-account пилота.
- Положительный LDAPS bind/RootDSE и WinRM HTTPS/CIM; реальные auth/cert/DNS failures
  и timeout process tree в домене/workgroup. Адаптеры написаны, их сетевой успех не заявлен.
- Две нативные remote GUI в реальной Windows-сессии/на двух ПК; native screenshot M2,
  полная клавиатура/screen reader/High Contrast/physical DPI matrix.
- CRL/OCSP hard-fail, certificate lifecycle, offline backup/restore/identity recovery,
  release/MSVC/installer/signature/dependency-license-security audit, большие объёмы/
  inventory paging/retention, pinned server-side preview-session и production load/SLO.
  Нет проверенной offline break-glass recovery для истёкших всех Admin tokens.

Никаких установок ПО на цели, обхода TLS, firewall/TrustedHosts/UAC/domain изменений,
реальных сетевых connection probes или системных SCM изменений не выполнялось.
Изменялись только ACL новых собственных test-каталогов. Старые проекты/данные не изменены.

## Следующая приёмка на доступном доменном ПК

Не вводить пароли/token в чат. Сначала согласовать **конкретные** DNS targets,
размещение сервера, service identity и разрешённые read-only операции.

1. Проверить DNS/time, LDAPS/WinRM HTTPS listeners, SAN/chain/expiry и доменную политику
   Kerberos/NTLM. Никаких auto-fix. Если NTLM запрещён, не обходить запрет: рассмотреть
   сервер на доступном доменном ПК и отдельный Kerberos-only профиль в M3.
2. Явно разрешённый пилот: один server + две consoles, персональные tokens. Проверить
   Viewer deny/expiry/revoke, общий реестр и conflict при одновременной правке.
3. AD Read account → только согласованный DC/636 RootDSE; отдельный Execution account →
   только согласованный ПК/5986 CIM Caption. Ошибки не превращаются в success.
4. При закрытых consoles restart службы/Windows с той же account/profile: secrets
   доступны. В отдельной **тестовой копии**, не production vault, другая identity должна
   дать blocking error без сброса данных/fallback. Проверить queued restart/cancel.
5. Зафиксировать результаты, точные версии ОС/среду/ограничения. До этого M2 acceptance
   в `roadmap.md` остаётся открытой. Инструкция: [m2-server.md](m2-server.md).

## Доменная проверка 24 ноября 2026

Windows 11 Pro, сборка 26300, PowerShell 7.6.9, Windows PowerShell 5.1,
Rust 1.100.0 GNU. Проверки связности выполнены под текущей доменной учётной
записью с Negotiate и проверкой TLS. Имена целей согласованы с владельцем.

| Проверка | Результат |
|---|---|
| Согласованный контроллер домена, LDAPS 636 | Прочитан `defaultNamingContext` из RootDSE |
| Согласованный сервер, WinRM HTTPS 5986 | Прочитан `Win32_OperatingSystem.Caption` на Windows Server 2025 Standard |
| Текущая станция, WinRM HTTPS 5986 | Сбой установления TLS (`0x803381ba`). Listener указывает на действующий сертификат с DNS-именем станции, но HTTP.sys не показывает SSL-привязку на 5986 |
| Полная проверка `tools/check-core.ps1` из PowerShell 7 | Форматирование, Clippy, 96 тестов и сборка прошли. HTTPS, роли, отзыв токена и DPAPI после перезапуска процесса проверены на временных данных |

Исправлены два сбоя сервера. Дочерний Windows PowerShell загружает свои модули
после удаления унаследованного `PSModulePath` в его окружении. CIM-адаптер использует
`New-CimSessionOption -UseSsl`, который создаёт параметры WSMan HTTPS в Windows
PowerShell 5.1. Системные настройки WinRM, HTTP.sys и службы не менялись.

Остаются проверки через сохранённые профили AD Read и Execution в Orkestr,
вторая физическая консоль, отдельная Windows identity, SCM/reboot и PKI/revocation.
