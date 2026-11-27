# План испытаний и критерии выпуска

Документ описывает план испытаний Orkestr. Сам план не означает, что проверки пройдены.
Последний серверный инкремент M3–M5: [m3-m5-verification.md](m3-m5-verification.md).
Фактически выполненные локальные проверки первого инкремента M1 вынесены отдельно:
[m1-verification.md](m1-verification.md).
Итог всей M1 с нативной консолью: [m1-console-verification.md](m1-console-verification.md).

## 1. Unit / contract (без изменения сети)

- Version parsing: 2.9/2.10, v-prefix, four-part, prerelease, равенство после normalize,
  неизвестный формат, downgrade, одинаковый номер с новым hash.
- JSON Pointer escapes, missing/null, HTML 0/multiple selectors, attribute/text,
  relative href, regex capture и запрещённые конструкции/лимиты.
- URL template injection, invalid hosts/ports, redirects, SSRF/private ranges,
  явное internal repo exception, DNS rebinding, auth-header stripping.
- Scheduler с подставными часами: UTC/DST, окно через полночь, jitter,
  concurrent jobs, default site, starvation, global/site/device caps.
- Approval привязано к revision/hash, нельзя approval/replay для изменённого пакета.
- Auth/RBAC на каждом endpoint, включая собственный job, secrets и shutdown.
- 409 edit conflict, request/body/output limits, pagination/cursors, idempotency.

## 2. Локальные интеграционные

- SQLite transaction rollback, migrations, durable leases, failure injection,
  duplicate idempotency key, unknown execution после simulated crash.
- Реальный CurrentUser DPAPI round-trip с кириллицей и спецсимволами;
  недоступность для другой identity, повреждённый blob, отсутствие профиля.
- Отсутствие пароля/token в argv, логах, API list/export/error, crash diagnostics.
- TLS bad CA/name/expired cert, revoked token, ротация CA/token.
- Downloader на локальном управляемом HTTPS fixture: ETag/304, 429,
  timeout, truncated response, size cap, .part, hash mismatch, redirect policy.
- Неподписанный/не тот publisher, disk-full, read-only cache, cleanup pinned hashes.
- Installer arguments mock, Unicode stdout/stderr, bounded output, quoting,
  exit 0/3010/failure, local process cancellation без ложного remote success.

## 3. Разрешённая тестовая сеть (обязательна)

Матрица: domain PC, workgroup PC с доверенным HTTPS, offline PC,
ПК с неверным сертификатом, ПК с неверным паролем, хотя бы две консоли.

- Реальный AD discovery/OU/page>1000/rename/disabled/exclusion/no-delete-on-error.
- Kerberos и права выделенной service identity; workgroup Negotiate HTTPS,
  политика NTLM/UAC без обходов.
- Scan + package copy + hash + безопасная тестовая установка + detection.
- Сетевая нагрузка нескольких jobs: ни один site/device/global cap не превышается.
  Byte-rate обещать лишь после измерения адаптера, включая overhead/retries.
- Approval → pilot → soak → rollout; pause при failure threshold.
- Потеря связи во время install, reboot ПК, server crash/restart;
  unknown состояние не вызывает слепую повторную установку.
- SCM install/start/stop/reboot с закрытым UI, смена identity и повторный ввод секретов.
- Backup/restore на том же сервере и документированный повторный ввод на новом.

Любые установки, изменения firewall/UAC, перезагрузки и доменные проверки требуют
явного разрешения владельца тестовых машин.

## Ручной пилот M3

Проверяйте M3 до M5 на отдельной базе, тестовой OU и разрешённых Windows-целях.
Для управляемой установки выберите отдельную ВМ со снимком и доверенный тестовый MSI.

1. Запустите сервер на отдельной базе и подключите консоль с токеном Admin.
   В **Настройки → Добавить подключение к домену или ПК…** настройте AD для
   тестового домена и WinRM для тестового ПК.
2. Откройте **Компьютеры → Добавить компьютеры → AD · подразделение**.
   Укажите DN тестовой OU. Сверьте строки, примените предпросмотр, проверьте
   повторный запрос и сохранение идентификатора при переименовании тестового объекта.
3. Из карточки тестового ПК запустите скан. Сравните ОС, RAM, ПО и вкладку
   «Провайдеры» с данными машины. При обрыве проверьте дату последнего сохранённого снимка.
4. Для установки выполните `orkestr-server stage-msi --data <каталог_сервера> --file <абсолютный_путь_к_MSI>`.
   В разделе **Пакеты → Развёртывание пакетов** зарегистрируйте хеш, ProductCode
   и ожидаемую версию `DisplayVersion`. Создайте план для ВМ, проверьте, что он
   ждёт одобрения, и отдельно одобрите его. Сверьте итог, код выхода и установленную версию.
5. На ВМ проверьте отмену до запуска и сценарий обрыва во время установки.
   Неопределённый исход сохраняет блокировку ПК. Сверка читает версию, не отправляя
   установщик повторно. Код 3010 показывает необходимость перезагрузки.
6. Отдельно проверьте EXE на ВМ со снимком: `stage-exe`, ревизия через API с явными
   тихими параметрами и GUID системной записи, план и одобрение. Сверьте передачу
   кавычек/пробелов, quiet/no-restart производителя, точную версию и код выхода.
   Обрыв/отсоединённый installer без подтверждения не должен давать success или автоповтор.
   [Контракт и ограничения EXE](m3-managed-installers.md).

В коде M3 проверены сценарии на временной SQLite и имитации WinRM. На согласованном
доменном сервере выполнено чтение названия ОС через WinRM HTTPS/Kerberos под текущей
учётной записью. Реальный запрос тестовой OU, скан с сохранёнными учётными данными,
выкладка MSI и рабочая группа остаются в матрице пилота.

Скан ПО читает системные записи и уже загруженные пользовательские реестры.
Переменные среды показывают имена, автозагрузка показывает записи без командных строк.
Эти разделы помечаются как частичные.

## 4. UI и справка

- Оба HTML-макета: тема, маршруты, поиск, формы, source fixture, очередь, ошибки.
- Будущий egui: keyboard-only, visible focus, High Contrast, screen reader feasibility,
  Windows DPI 100/125/150/200%, resize, длинные русские строки, light/dark/system.
- Empty/loading/partial/error/offline/permission-denied; не рисовать «всё хорошо» по умолчанию.
- Каждый опасный шаг явно показывает ПК, учётную запись, пакет/revision и последствия.
- F1 открывает локальный help; работает без Internet, ссылки и поиск не сломаны.
- Справка соответствует поставляемому build; пример не выдан за production-команду.

## Ворота выпуска

1. Повторяемая release build, тесты, dependency audit и license manifest.
2. Migration dry-run и восстановление проверены на копиях настоящих форматов.
3. Pilot на разрешённой сети, измерения нагрузки, подписанная матрица результатов.
4. Installation/upgrade/uninstall не удаляют data/credentials без отдельного согласия.
5. Нет критических открытых проблем auth/TLS/secrets/duplicate execution.
6. Известные ограничения опубликованы. Для недоказанного теста статус Not run,
   не «Passed, потому что код компилируется».
