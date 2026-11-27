# Проверка M1.1

Исторический отчёт первого инкремента. Текущий повторный прогон всей M1:
[m1-console-verification.md](m1-console-verification.md).

Дата: 2026-11-06. Только локальные синтетические данные; исходные приложения не запускались.

## Среда

- Windows, `rustc 1.99.1 (58cf8f9c3 2026-10-13)`, target `x86_64-pc-windows-gnu`.
- C/linker: существующий GCC из
  `C:\Users\it\Documents\Craft Anything\pdqinventoryconsole-2\app\tools\w64devkit\bin`,
  использован только как внешний инструмент компиляции. Не копировались его файлы,
  старые `target`, EXE, runtime или keys; исходные приложения/скрипты не запускались.
- В проекте нет абсолютных linker paths. `.cargo/config.toml` задаёт GNU
  `link-self-contained=yes`; MSVC Build Tools здесь не обнаружены и не проверялись.
- Первоначальная GNU-сборка без этого флага остановилась на `cannot find -lgcc_eh`.
  После настройки выполнен полный повтор проверок ниже. Это не скрытая установка SDK.

## Выполнено

```powershell
.\tools\check-core.ps1 -Gnu -CompilerBin 'C:\Users\it\Documents\Craft Anything\pdqinventoryconsole-2\app\tools\w64devkit\bin'
```

Скрипт последовательно выполнил:

| Команда (с `cargo +stable-x86_64-pc-windows-gnu`) | Результат |
|---|---|
| `fmt --all -- --check` | PASS |
| `clippy --workspace --all-targets --locked -- -D warnings` | PASS, без warnings |
| `test --workspace --locked` | **28 passed, 0 failed** |
| `build --workspace --locked` | PASS, debug CLI собран |

Разбивка тестов: `domain/tests/collections.rs` — 11,
`storage/tests/core.rs` — 6, `storage/tests/import.rs` — 9,
`cli/tests/smoke.rs` — 2. Unit/doc test harnesses без тестов не включены в число 28.

Что фактически проверено:

- Numeric versions, 8/16 GiB ±1 byte, все numeric operators, AND/OR/Unknown,
  stale/future scans, case-insensitive имена и software predicates на одной записи.
- Library reference fixture меняет состав; approved не подменяет latest known;
  unmapped application остаётся Unknown. Сетевой библиотеки нет.
- Static membership, folder union без дублей, nested tree в 1024 папки,
  запрет циклов/конвертации непустой папки, Unknown не дублируется в members.
- Schema init/reopen, отказ от чужой/новой/повреждённой схемы, immutable scans/revisions,
  latest scan head, AD rename/alias identity, сохранение подробностей и static DeviceId.
- Два локальных connections: stale expected revision не затирает новую правку.
  Это не тест двух production-консолей/API.
- Dry-run synthetic legacy JSON/SQLite, byte equality источника до/после,
  отсутствие WAL/SHM у backup source, идемпотентность ledger после restart,
  blocking unsupported steps/rules/GUID conflicts/unresolved targets,
  отказ от live WAL и изменившихся source/target.
- Failure injection через тестовый SQLite trigger посреди apply: пакет, ledger и
  generation откатываются, после снятия fixture trigger batch успешно применяется.
- Реальные subprocess CLI smoke: help, запрет implicit create/overwrite, пустой list,
  dry-run без записи, apply с новым backup, list после restart; backup восстановлен
  в новый путь и показывает исходное пустое состояние. Group create/update/preview/delete,
  конфликт ревизии и отсутствие синтетического секрета в parse error stderr.
- История/токен из JSON fixture не попадают в report. Не заявляется универсальное
  удаление секретов из произвольных legacy command bodies или Environment.

Дополнительно:

```powershell
py -3 tools/validate.py
node --check mockups/app.js
node --check mockups/groups.js
node --check docs/help.js
```

PASS: 4 HTML-страницы, локальные ссылки/anchors, JSON, CSS tokens, offline JS guard;
все три Node syntax checks — exit 0. HTML/JS не менялись.

## Not run / не реализовано

- Release/MSVC/другая машина, dependency security/license audit, подписанный дистрибутив.
- Импорт настоящих данных владельца и полная обратимая миграция всех старых сущностей.
- egui render/UI smoke/DPI/keyboard, измерение отзывчивости UI на 50 ПК и нагрузочные SLO.
- Служба/SCM, API/two-console/RBAC/TLS/DPAPI, сеть/AD/WinRM/CIM, installs/reboot,
  скачивания, approval/jobs/scheduler. Это не запускалось и не разрешалось этой задачей.

M1 целиком не закрыт. Результат не означает production readiness.
