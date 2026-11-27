# Библиотека приложений и обнаружение обновлений

## Не одно поле «ссылка»

У приложения разделить:

1. **Обнаружение версии:** где и как получить номер новой версии.
2. **Разрешение URL:** какой конкретно установщик соответствует версии/каналу/архитектуре.
3. **Верификация:** подпись, hash, размер, издатель.
4. **Установка и обнаружение на ПК:** silent args, acceptable codes, detection.
5. **Автоматизация:** check/download/approval/pilot/deploy.

Это позволяет API версии и CDN установщика находиться на разных разрешённых hosts.

## Поддерживаемые источники V1

| Тип | Настройки | Проверка в редакторе |
|---|---|---|
| JSON API | HTTPS URL, GET, credential reference, JSON Pointer, optional version regex | HTTP status, ограниченный безопасный fragment, исходное/нормализованное значение |
| HTML | HTTPS URL, CSS selector, text либо attribute, optional regex | Число совпадений, выделенный текст/атрибут, extracted value |
| Текст | HTTPS URL, линейно ограниченный regex/capture | Match count, capture, версия |
| Вручную | Версия + локально импортированный artifact | Проверка файла/hash и detection |

JSON Pointer RFC 6901 выбран для простоты (`/releases/0/version`), не произвольный
eval и не JavaScript expression. Нет исполнения пользовательского кода ради номера версии.
Пагинация/фильтры prerelease/архитектуры описываются явной конфигурацией источника.
Если API требует POST или сложного OAuth — это отдельный адаптер после V1.

CSS selector: например `.release-current .version`; attribute пусто = textContent.
При 0 или >1 совпадениях ошибка по умолчанию. Режим выбора среди нескольких должен
быть явным и детерминированным. Скрипты/iframes ресурсы страницы не запускаются.
Антибот/login/JS-only страницы не обходятся: нужен API либо ручной источник.

## Регулярные выражения

- Backend Rust `regex` с ограничением длины паттерна/тела/числа matches, без
  lookaround/backreferences; это **не** произвольный JS/.NET regex engine.
- Именованная группа `(?P<version>\d+\.\d+(?:\.\d+)?)` или явно выбранная
  числовая группа; результаты пустых/неоднозначных совпадений блокируют pipeline.
- Извлекать версию из текста/имени файла regex-ом, а HTML сначала DOM parser.
- Нормализация `v2.4.1` → `2.4.1`, но сохраняется raw. Не сравнивать лексикографически:
  `2.10 > 2.9`; схема сравнения SemVer / numeric segments / publisher-specific.
- Непонятная версия, downgrade или смена канала не разрешают unattended update.

## URL загрузки: два режима

**Шаблон:** `https://downloads.example.test/atlas/{version}/atlas-{version}-{arch}.msi`.
Разрешённые переменные version, version_raw (только явное разрешение), arch, channel.
Значения валидировать и encode в зависимости от сегмента URI; нельзя подставить новый host.

**Извлечение ссылки:** download-page URL → CSS `a.release-download` → `href` →
разрешить относительный URI через base URL → regex-фильтр пути/имени → capture version
должен совпасть с discovered version → ровно один кандидат нужной arch/channel.
Пример фильтра: `/atlas-(?P<version>\d+\.\d+\.\d+)-x64\.msi$`.

Обязательно показать final URL после redirect, источник номера и причину выбора.
Плавающий `/latest` можно скачать только в кандидат, затем закрепить hash и metadata;
одобрение не должно «переехать» на другой файл по той же ссылке.

## Pipeline

```text
Configured → Checking → Candidate → DownloadQueued → Downloading (.part)
  → Quarantined → Verified → AwaitingApproval → Approved → Pilot → Eligible
                    ↘ Rejected / SourceError / VerificationFailed
```

«Скачан» не означает «одобрен», а «одобрен» не означает «установлен».
Новый hash при той же версии создаёт новый candidate и требует нового одобрения.
Release evidence: timestamp, source revision, URL, response hash, extracted raw value,
normalized version, redirect chain без секретных headers. Retention ограничить.

## Пример конфигурации (проектируемый формат, не рабочий import)

```json
{
  "schema_version": 1,
  "name": "Atlas Editor (учебный пример)",
  "channel": "stable",
  "architecture": "x64",
  "version_source": {
    "kind": "json",
    "url": "https://releases.example.test/api/latest",
    "json_pointer": "/version",
    "normalization": "numeric_segments"
  },
  "download": {
    "kind": "template",
    "url": "https://downloads.example.test/atlas/{version}/atlas-{version}-x64.msi",
    "allowed_hosts": ["downloads.example.test"],
    "max_bytes": 536870912
  },
  "automation": {
    "check": true,
    "download": false,
    "approval": "manual",
    "deploy": false
  }
}
```

Домен `.test` намеренно не рабочий. Это не реальный пакет поставщика.

## Загрузка

Одна загрузка артефакта на сервер, reuse по hash для всех ПК. Ограниченный pool,
rate limit, proxy policy, timeout, max bytes, Content-Type + file magic, free-space guard.
ETag/Last-Modified для check, backoff на 429/5xx и Retry-After.
Resume только с Range и подтверждением прежнего ETag/If-Range; иначе заново в .part.
После stream hash + signature verification — atomic publish, незавершённые файлы
не доступны deployment. Очистка кэша не удаляет hash, на который есть queued/running job.

## Detection на компьютерах

Не искать только DisplayName contains. Правило сочетает publisher, product code/registry,
архитектуру и scope; при необходимости file version. Пример исходной Inventory
не собирает user-installed apps — в V1 это отдельное ограничение, не «ПО отсутствует».
Если скан устарел или провайдер упал — Unknown, а не Compliant/NotInstalled.
После установки выполнить detection. 3010 = результат с требованием reboot,
не ошибка и не автоматическое разрешение перезагрузить пользователя.

## Автополитика

По умолчанию manual approval. При включении unattended: разрешённый app/channel,
правила доверия, неизменная signature identity, успешный пилот/выдержка, окно,
порог ошибок, лимиты сети. Изменение источника/скрипта/аргументов сбрасывает одобрение.
Auto-download может быть включён без auto-deploy. Все effective значения и источник
наследования должны быть видны до сохранения.
