# Проверка завершения M1

Дата: 2026-11-06. Только локальная проверка на явно синтетических данных.
M1 реализован, но это **не production readiness и не разрешение на rollout**.

## Сборка и тесты

Windows, Rust `1.99.1`, target `x86_64-pc-windows-gnu`; egui/eframe `0.31.1`.
Использован существующий GCC как внешний compiler, без копирования toolchain/target/
runtime/EXE/keys соседних проектов и без изменения исходных приложений.

```powershell
$env:TMP='C:\Temp\opencode'
$env:TEMP='C:\Temp\opencode'
.\tools\check-core.ps1 -Gnu -CompilerBin 'C:\Users\it\Documents\Craft Anything\pdqinventoryconsole-2\app\tools\w64devkit\bin'
```

| Проверка (`cargo +stable-x86_64-pc-windows-gnu`) | Результат |
|---|---|
| `fmt --all -- --check` | PASS |
| `clippy --workspace --all-targets --locked -- -D warnings` | PASS, без warnings |
| `test --workspace --locked` | **43 passed, 0 failed** |
| `build --workspace --locked` | PASS, debug console и CLI |

Разбивка: domain 11, storage core 6 + import 9 + upgrade/review 3,
application 5, console 7, CLI smoke 2. Пустые unit/doc harnesses не включены в 43.

Проверено дополнительно к [историческим тестам ядра](m1-verification.md):

- Bounded command/event queues, non-blocking backpressure, явное создание новой БД,
  отсутствие implicit create/overwrite и stale revision overwrite.
- Worker preview и дерево используют одинаковые generation **и timestamp**;
  старые request tokens не подменяют новый preview. Preview не сохраняет draft.
- Настоящие egui pointer/text events (headless Context, не вызов одного save-handler):
  открыть редактор, дождаться preview, сохранить группу и открыть SQLite повторно;
  отмена следующей группы не меняет БД. Import UI: выбрать формат, ввести source,
  подтвердить копию, dry-run без записи, подтвердить apply, ввести новый backup path,
  применить и проверить источник/backup/пакет.
- Keyboard events: Ctrl+N, Escape, Ctrl+F/focus, F1. Render всех страниц и F21-вкладок
  в light/dark при zoom 100/125/150/200%, сериализация персональных настроек.
- Контраст ink/muted/accent относительно обеих surface/bg: WCAG ratio >=4.5.
  Это тест указанных токенов, не полная accessibility certification всех OS controls.
- Additive v1→v2 upgrade с отдельным согласованным pre-upgrade backup, readable v1 backup.
- Package lineage при одинаковом явном origin и разделение независимых origins.
  Новые revisions immutable, описание/исходная папка сохраняются.
- Schedules/target lists/history только review, enabled=false, recurrence/next/was_enabled
  сохранены. Config/token/password/history outputs fixtures не попадают в review/report.
  Непереносимый rounded-RAM rule остаётся выключенным, не меняет смысл активной группы.

## Native render smoke

```powershell
.\tools\smoke-console.ps1 -OutputDirectory .\test-results\m1-native-03
```

PASS: создана новая папка с собственными legacy-копиями, импортированы **50 учебных ПК**,
50 сканов, 1 ручная группа и 1 пакет; добавлены 10 тестовых папок/умных групп.
4 review records: старое RAM-правило, target list, выключенное расписание и история.
Hashes обеих исходных копий сверены до/после. Повторный CLI list: 50 ПК,
11 групп, 1 пакет, 4 review; generation=12. Capture не сохраняет новые группы.

Получены реальные eframe/OpenGL screenshots 1280×820 (не нарисованный HTML):

- light/dark overview;
- light editor, device card, import form и выбранная RAM-группа;
- dark packages и review;
- light editor и dark settings с zoom 200%.

Пути: `test-results/m1-native-03/*.png`, повторный list:
`test-results/m1-native-03/restart-list.txt`. Это локальные воспроизводимые artifacts,
не входят в release. Учебная версия Chrome 2.9/2.10 — не реальный текущий релиз Google.
В раннем smoke замечены недоступные нижние кнопки при zoom 200%; боковая панель,
settings/help получили прокрутку, затем тесты и native smoke повторены.
После перевода вспомогательных надписей на проверенный muted token повторены все
43 теста/fmt/clippy/build и отдельный нативный capture выбранной группы:
`test-results/m1-native-03/final-group-AA.png` — PASS.

## Локальное измерение, не SLO

```powershell
cargo +stable-x86_64-pc-windows-gnu test -p orkestr-application fifty_computers_preview --locked -- --nocapture
```

PASS, один отдельный debug-прогон: worker round-trip preview по 50 небольшим fixture
records — **1.5694 ms**. Не измерение сети, UI frame time, production нагрузки или
гарантия для больших данных; реальные SLO определяются отдельно.

## Материалы

`py -3 tools/validate.py` — PASS (4 HTML-страницы, ссылки/anchors, JSON, CSS tokens,
offline JS guard). `node --check mockups/app.js`, `node --check mockups/groups.js`,
`node --check docs/help.js` — PASS, exit 0.
HTML-макеты/знак не переписывались; MIT attribution сохранён.

## Not run / следующие этапы

- Настоящие рабочие копии владельца, production migration/restore drill, большие объёмы.
- Release/MSVC/другая Windows-машина, подпись, installer, dependency/license/security audit.
- Полная Tab-only, screen reader/High Contrast, физическая Windows DPI/мультимониторная
  матрица. UI zoom и headless keyboard tests не выданы за её замену.
- Служба/SCM/API/RBAC/TLS/DPAPI и две серверные консоли — M2; провайдеры/WinRM/CIM/AD,
  installs/reboot — M3; production library discovery/matching/download — M4.
- Сеть, установка ПО, reboot, firewall/domain/service modifications **не выполнялись**.

Legacy commands/Environment могут содержать чувствительные данные: БД/backup приватны,
произвольные definitions не являются автоматически очищенным публичным export.
