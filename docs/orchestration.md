# Оркестрация и защита сети

В V1 нет агентов: сервер дозирует копирование пакетов и запуск WinRM на **целевых ПК**.
Принцип переносим на агенты в будущем, но agent polling/leases сейчас не выдаются за готовые.

## Три независимых ресурса

1. Интернет → repository сервера: download pool и egress/download budget.
2. Сервер → ПК: transfer slots + реальный byte-rate limit по площадке и глобально.
3. ПК выполняет installer: install slots, per-device exclusive mutation lock.

Один общий `concurrency=8` из Deploy не покрывает эти ограничения.
Все задания и retries делят **единые** лимиты, не получают свои независимые пулы.

## Предлагаемый осторожный старт

| Настройка | Черновое значение |
|---|---|
| Internet downloads | 1 |
| Endpoint transfers global | 2 |
| Endpoint transfers per site | 1 |
| Installs global | 2 |
| Изменяющих задач на ПК | 1 |
| Jitter старта | 0–120 секунд |
| Retry | 3 попытки, exponential backoff + jitter; auth errors не повторять вслепую |
| Режим «строго по одному» | transfers=1 и installs=1, без overlap между целями |

Это не реальные benchmark и не согласованные параметры. До настройки sites все
неразмеченные ПК относятся к общей default site; лимиты не обходятся пустым site_id.
Для требования «совсем не одновременно» есть строгий режим, иначе допускается
ограниченный параллелизм согласно бюджету.

## Состояния задания на цель

```text
Planned → WaitingApproval / WaitingWindow / WaitingQuota / WaitingCredential
        → Preflight → Transferring → Verifying → Installing → Detecting
        → Succeeded / SucceededRebootRequired / Failed / NeedsReconciliation
             ↘ RetryScheduled / CancelRequested / CancelledBeforeStart
```

Каждая попытка неизменяема после завершения; retry создаёт новый attempt.
Читаемые статусы ожидания показывают причину и время следующей оценки, не «зависло».

## Планирование

1. При создании job материализовать список DeviceId и immutable package revision/hash.
   Динамическая коллекция не должна незаметно расширять уже одобренную волну.
2. Исключить disabled, запрещённые scopes, fresh detection=already installed;
   неизвестное detection требует скана/проверки, не автоматической установки.
3. Разделить pilot и production. Проверить дату/часовой пояс окна, DST policy,
   время окончания; новых целей после закрытия окна не начинать.
4. Получить durable lease через транзакцию SQLite. Лимиты, device lock, site quotas
   должны резервироваться атомарно; worker не получает задачу без квоты.
5. Transfer phase отдельно от install. Передавать из server cache, hash на цели,
   затем installer; direct winget/download на ПК выключен в managed-channel режиме.
6. Справедливость между jobs/sites; priority не должна навсегда вытеснять обычные задачи.
7. Jitter дополняет лимит, но не заменяет его. Состояние retry и `next_attempt_at`
   сохраняется; рестарт не начинает все попытки одновременно.

## Честный контроль скорости

`Copy-Item -ToSession` из старого engine **не имеет точного byte-rate limiter**.
В V1 сначала гарантировать ограничение количества передач, waves и preload server cache.
Для обещания Mbps нужен новый контролируемый streaming/chunked-transfer адаптер
или проверенная реализация BITS с учётом политики Windows и аутентификации.
Нельзя выдавать поле «10 Мбит/с» в UI за гарантию, если транспорт его не соблюдает.
Передача chunked через WinRM должна учитывать overhead, hash, resume и ACL staging.
Пока rate-adapter не готов, поле помечено unsupported, доступен только concurrency.

## Ошибки и рестарт

- Лимиты сети/ошибок превышены → pause wave, не стоп всех чужих заданий.
- Порог пилота: считать только завершённые известные результаты; unknown не success.
- Offline → bounded retry до TTL и окна; ICMP failure не доказывает offline.
- Authentication failed → pause credential scope, не вызвать блокировку AD перебором.
- Expired lease при рестарте → NeedsReconciliation, затем detection/receipt probe.
  Не запускать installer автоматически второй раз.
- Без агента невозможно обещать exactly-once на произвольном detached процессе.
  At-least-once delivery + idempotency keys + detection, неизвестное — ручная сверка.
- Cancel до старта = CancelledBeforeStart. После запуска = CancelRequested,
  транспорт остановлен/отправлена отмена, результат installer может остаться неизвестным.
- Перезагрузка только отдельной разрешённой политикой; не из кода 3010 автоматически.

## Что видеть в интерфейсе

Очередь: цель, приложение/version/hash, phase, site, полученные квоты, причина ожидания,
attempt, время, effective policy. Предпросмотр до запуска показывает оценку волн,
не обещанное время завершения. Общий расход сети измеряется отдельно и не подменяется
процентом целей. Смена лимита влияет на выдачу новых работ, не убивает installer на полпути.
