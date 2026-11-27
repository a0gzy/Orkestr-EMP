# M1 — локальное ядро и консоль Orkestr EMP

Реализация M1 без массового исполнения, **не production-ready приложение**.
Никаких сетевых запросов, PowerShell, сканирования, установки/загрузки ПО, службы,
учётных данных, расписаний или автоматического заполнения учебными данными.
HTML A не изменён. Нативная egui-консоль сохраняет знак, палитру и боковую навигацию:
[m1-console.md](m1-console.md).

## Структура и поведение

- `crates/domain`: отдельные UUID-типы, реестр, immutable scan/revision models,
  numeric-segment versions (`2.10 > 2.9`), типизированное дерево И/ИЛИ, сравнения
  `= != > >= < <=`, RAM Unknown, текст, software conditions на одной записи ПО.
- `crates/storage`: SQLite v2 с application ID, foreign keys, WAL, busy timeout,
  транзакциями; scans и software rows отдельно, current scan head, alias/AD identity,
  optimistic group revisions, imports ledger/UUID map, согласованный backup.
- `crates/application`: co-located service, один worker, bounded command/event queues
  по 8 сообщений. I/O/preview/импорт вне UI thread; stale preview tokens не заменяют новые.
  Счётчик/список используют одну generation и время оценки.
- `crates/console`: egui 0.31.1, группы/дерево/редактор И/ИЛИ, карточки, темы и zoom,
  импорт и read-only review. Никаких seed-данных при обычном запуске.
- `crates/cli`: явные локальные операции. Не графическая общая консоль и не сервер.
  Список/preview/dry-run открывают целевую БД read-only; новый файл создаёт только `init`.
- Папки — union потомков без повторов, без неявного наследования фильтра.
  Запрещены циклы, несуществующие родители и превращение непустой папки в выборку.
  Удаление группы не удаляет устройства/сканы. При сохранении требуется expected revision.
- Preview содержит generation, UTC evaluation time, полный набор совпавших records/ПО
  и отдельные Unknown IDs. Старый preview не меняется при новом скане.
- RAM хранится в байтах installed physical memory; **GiB = 1 073 741 824 байта**.
  В импортируемом Inventory берётся сумма `MemoryModules[].Capacity` при отсутствии
  ошибки этого провайдера. `MemoryGB` (округлённый usable/total) — не замена.
- Свежесть в CLI: 24 часа; будущий scan тоже Unknown. Это временная настройка M1.
  Неполное/устаревшее ПО не считается отсутствующим или актуальным.
- VersionOperand поддерживает literal и late-bound library reference. Только fixtures
  могут сейчас предоставить library reference; `latest known` и `approved` раздельны.
  M4 добавит production matching/discovery. SemVer prerelease пока Unknown.

## Сборка

Нужны Rust stable и собственный C/linker toolchain для bundled SQLite: MSVC Build
Tools + Windows SDK, либо GNU Rust + доступный MinGW-w64 GCC. Не копировать toolchain,
`target` или бинарные файлы соседних приложений в Orkestr.

```powershell
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo fmt --all -- --check
cargo build --workspace --locked
```

При GNU-компиляторе выбирайте `cargo +stable-x86_64-pc-windows-gnu ...` и добавьте
каталог **своего** GCC в PATH текущей shell. Проект не меняет глобальные настройки
Rust, PATH, политики PowerShell или состояние служб.

Повторяемая проверка без ручного перечисления команд: `tools/check-core.ps1`.
Для GNU есть параметры `-Gnu -CompilerBin <каталог-своего-GCC>`; PATH меняется только
на время вызова и восстанавливается. `Cargo.lock` включён в workspace.

## Явный запуск (без автодействий)

Команды ниже создают только локальные файлы в заранее созданном рабочем каталоге:

```powershell
cargo run -p orkestr-cli --locked -- --help
cargo run -p orkestr-cli --locked -- init .\data\orkestr.sqlite
cargo run -p orkestr-cli --locked -- list .\data\orkestr.sqlite
```

Нативный запуск: `target\debug\orkestr-console.exe` или `cargo run -p orkestr-console --locked`.
Без аргументов — явный выбор существующей БД/создание пустой. `--database <путь>`
открывает существующий локальный файл. `data` для CLI заранее создаёт администратор;
`init` отказывается открывать существующий путь.
Файл БД держать на локальном диске. Это не сетевой client mode и не multi-console API.

### Правило и группа

Пример **выражения**, а не встроенная коллекция (сохранить в собственный JSON-файл):

```json
{"And":[{"Ram":{"op":"Ge","bytes":8589934592}},{"Ram":{"op":"Lt","bytes":17179869184}}]}
```

```powershell
cargo run -p orkestr-cli --locked -- preview-rule .\data\orkestr.sqlite .\range.json
```

Для RAM `>=16`, `>=8`, `<8` использовать `Ram` с `Ge/Ge/Lt` и соответствующими байтами.
Unknown: JSON-строка `"RamUnknown"`. `And`/`Or` поддерживают вложенность до 16, максимум
256 узлов. Software identity + publisher/channel/architecture/version — один predicate,
не отдельные независимые existential predicates на разных записях ПО.

Группа — JSON с собственным UUID, `parent_id` (UUID папки или null), `name`, `revision`
(при create игнорируется), `kind`: `"Folder"`, `{"Static":["device-uuid"]}` либо
`{"Dynamic":<выражение>}`. Это дополнительный CLI-вариант;
в нативной консоли есть визуальный редактор без ручного JSON.

```powershell
cargo run -p orkestr-cli --locked -- save-group .\data\orkestr.sqlite .\group.json new
cargo run -p orkestr-cli --locked -- save-group .\data\orkestr.sqlite .\group.json 1
cargo run -p orkestr-cli --locked -- preview-group .\data\orkestr.sqlite <collection-uuid>
```

Смена `parent_id` переносит группу. CLI `list` показывает пути, счётчики и revisions.
Preview показывает ПК, bytes RAM, дату snapshot, ПО и Unknown отдельно.

## Импорт только выбранных копий

Оригиналы **не выбирались и не импортировались автоматически**. Тесты создают
собственные синтетические копии фактических legacy-схем в временных каталогах.

1. Deploy: собственная закрытая копия `console.json`. Пакеты/10 известных типов steps
   сохраняются как неутверждённые immutable определения, без artifact hashes.
   Файлы installers не читаются. Списки целей, metadata истории и расписания сохраняются
   в read-only review с `enabled=false`; next/recurrence/was_enabled доступны для проверки.
   Цели списков/расписаний связываются с DeviceId, unresolved показаны отдельно;
   новые ПК из неразрешённых целей не создаются. History outputs/settings не переносятся.
2. Inventory: согласованный SQLite backup через штатный `VACUUM INTO`, не копия
   живого WAL-файла. При WAL-header/sidecars импорт блокируется. Источник открывается
   только read-only/query-only; target никогда не подставляется вместо source.
   Переносятся устройства, подробности сканов (F21), статические и поддержанные
   текстовые динамические группы. При неразрешённых целях, конфликте GUID/host,
   неподдержанном шаге импорт целиком заблокирован. Старые правила с непереносимым смыслом
   (включая rounded MemoryGB) сохраняются в выключенном review, а не становятся активным
   фильтром с другими результатами. Warning/review_items видны в dry-run.
3. Config, токены, DPAPI, логи и history outputs не читаются/не сохраняются из старых
   таблиц; из Deploy JSON они не извлекаются/не переносятся (весь файл читается для hash и parsing).
   Внутри legacy package commands или inventory Environment потенциально
   остаются чувствительные данные: БД/backup приватны, это **не scrubbed public export**.
   CLI не печатает raw details, command bodies или исходные parse/SQL exceptions.
4. Источник ограничен 64 MiB/100 000 сущностей, editor JSON — 1 MiB. Нет auto-discovery
   source paths, рекурсивного обхода старых данных, переноса installer paths или secrets.

```powershell
# Только dry-run; БД Orkestr уже создана через init.
cargo run -p orkestr-cli --locked -- import inventory .\data\orkestr.sqlite C:\MyCopies\inventory.sqlite
cargo run -p orkestr-cli --locked -- import deploy .\data\orkestr.sqlite C:\MyCopies\console.json

# Явное применение: сначала печать отчёта, затем НОВЫЙ согласованный rollback backup.
cargo run -p orkestr-cli --locked -- import deploy .\data\orkestr.sqlite C:\MyCopies\console.json --apply .\data\before-import.sqlite
```

Apply проверяет source SHA-256 и target generation. Вся запись/ledger/map одной
транзакцией; ошибка откатывает и ранее добавленные записи. Повтор того же hash
идемпотентен. После изменения source создаётся новый batch. В GUI «Происхождение»
связывает backup-файлы одного старого сервера: PackageId сохраняется, revision новая
и без approval. Разные серверы не объединять одним именем. Без явного происхождения
namespace определяется выбранным путём, не именем приложения или его IP.

При первом writable open v1 перед additive migration v2 создаётся новый согласованный
`*.before-v2-<uuid>.sqlite`. Чужие/неизвестные/повреждённые схемы отклоняются без замены.
Read-only open поддерживает v1/v2.

Rollback M1: закрыть все локальные handles Orkestr, сохранить текущий DB вместе с
его WAL для диагностики, восстановить согласованный backup в **новый рабочий путь**,
проверить `list` и далее использовать новый путь. Не копировать поверх живой БД.
Исходные приложения/данные нетронуты; никаких установок отменять не требуется.
Автоматической команды restore нет.

## За границами M1 / ограничения

- Проверки на одобренных настоящих копиях владельца не было: тесты создают собственные
  синтетические inputs фактических legacy-схем. Это не production migration sign-off.
- Полная автоматическая миграция/активация старых правил, расписаний, history outputs,
  внешних installer files и credentials не обещается. Review не исполняется.
- Совместная работа двух консолей через HTTPS API — M2. Проверка конфликтов локальных
  DB connections не выдаётся за готовый серверный режим.
- Полная физическая Windows DPI/High Contrast/screen reader матрица — M6;
  UI zoom/render smoke не заменяют приёмку на разных мониторах/системах.
- До распространения: лицензия нового кода, dependency/license/security audit, ACL и
  путь данных под службой. DPAPI, auth/TLS, jobs/approval/execution — следующие этапы.

Текущий прогон: [m1-console-verification.md](m1-console-verification.md).
Исторический результат первого инкремента: [m1-verification.md](m1-verification.md).
