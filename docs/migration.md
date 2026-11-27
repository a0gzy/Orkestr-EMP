# План миграции

Не открывать старые JSON/SQLite «на запись» из нового Orkestr.
Работать с копиями, сохраняя два исходных приложения как путь возврата.

## Источники

- Deploy: выбранная серверная `console.json` и связанные definitions/lists/schedules/history.
  Не путать production-файл с `checks`, test/selftest и `console.backup.json`.
- Inventory: согласованная копия `inventory.sqlite`, `config.json`, collections.
- Credentials: отдельная процедура, DPAPI старой identity не переносится простым rename.
- Installers: только собственные/разрешённые файлы, hash manifest; не `tools`, `target`,
  проприетарные reference EXE, decompilation или vendor catalogs.

## Этапы импорта

1. Пользователь явно выбирает источники и подтверждает, что это его рабочие данные.
2. Зафиксировать версии форматов, hashes и резервные копии. Inventory backup штатным
   SQLite способом; не копировать живую базу без WAL-consistent snapshot.
3. Dry-run: количество сущностей, ошибки, дубликаты, unsupported steps,
   пути файлов, пустые host/версии, проблемы секретов. Ничего не выполнять.
4. В новой БД создать UUID map: legacy key → new UUID; AD GUID первичен для
   reconciliation, host case-insensitive. Не объединять два ПК только по одному IP.
5. Import в транзакции, повторный запуск с тем же source hash идемпотентен.
6. Packages превращаются в неутверждённые immutable revisions. Пути старых installers
   проверяются на сервере, затем импортируются в artifact cache; не доверять клиентскому пути.
7. Коллекции/target lists связываются с DeviceId. Не найденные цели показать отдельно,
   не создавать молча ошибочные workgroup/AD identities.
8. **Все расписания, retries и auto-deploy после импорта отключены.**
   Времена/recurrence сохранить для review, но не запускать пропущенные задания.
9. История read-only, legacy outputs очищаются от известных секретов насколько возможно;
   разбор импортируемого вывода не исполняет содержимое.
10. Секреты ввести заново через Orkestr под окончательной service identity.
    Дополнительный same-identity importer допустим только после отдельного теста/подтверждения.
11. Сверить counts, выборки ПК/версий/steps, dry-run на read-only пакете, пилот.
12. Отключить старые службы/расписания вручную после подтверждения владельца,
    чтобы исключить двойной rollout. Исходные данные не удалять.

## Rollback

До массового запуска: stop выдачи Orkestr, остановить службу, сохранить диагностику,
вернуться к старым источникам без обратной записи в них. Выполненные установки
не отменяются возвратом базы; нужен отдельный vendor-supported downgrade/uninstall.
Не обещать универсальный откат стороннего ПО.

## Обновление самого Orkestr

Отдельный maintenance процесс: backup → drain queue → stop service → signature/hash
новой сборки → binary/schema upgrade → startup checks → read-only health → resume.
Новая schema несовместима с старым binary без проверенного downgrade пути.
План отката binary+database проверяется на тестовом сервере до выпуска.
