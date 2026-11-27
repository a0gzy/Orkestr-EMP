# Проверка текущих материалов

Дата: 2026-11-06. Это проверка **макетов и документации**, не тестирование будущего
Rust-приложения Orkestr. Сетевых подключений к ПК, AD, установок, reboot и изменения
служб/firewall не было. Исходные программы не изменялись.

## Выполнено

| Проверка | Результат |
|---|---|
| `py -3 tools/validate.py` | PASS: 4 HTML, локальные ссылки HTML/Markdown и anchors, JSON, CSS tokens, offline JS guard |
| `node --check mockups/app.js` | PASS |
| `node --check docs/help.js` | PASS |
| `node --check mockups/groups.js` | PASS |
| Браузерный `tools/browser-checks.mjs`, после добавления групп | **664 assertions passed, 0 failed**: 404 для A, 260 для B и справки |
| Макет B из `file://`, без HTTP-сервера | Локальное извлечение версии и URL успешно, внешних запросов нет |
| Настоящие Ctrl K / Escape | Поиск открывается, фокус на input, Escape закрывает dialog |
| Console errors макета B в file-режиме | 0 при проверенном сценарии |
| Цветовые semantic pairs A/B, light/dark | 32 измерения WCAG ratios в Chromium Canvas; минимум **5.63:1** для проверенных текстовых пар |

664 — число отдельных утверждений в собственной проверке DOM/геометрии/сценариев,
**не** 664 независимых unit tests и не число тестов Rust-кода.
В автоматизации используются same-origin iframe с реальной шириной viewport.

### Покрытие браузерного сценария

- A и B: 320 / 375 / 414 / 768 / 1024 / 1280 / 1920 CSS px, шесть разделов.
- Root horizontal overflow, размеры контролов, clipping контейнеров и labels.
- Окна подключения, профиля и поиска на 320 и 1280 px.
- Поиск ПК, переход из общего поиска к выбранной карточке.
- JSON Pointer / CSS selector / text regex на fixture, ошибка отсутствующего selector.
- Download href + regex extraction и сопоставление номера версии.
- Учебное одобрение → разблокировка preview плана, отсутствие реального запуска.
- Неверные/верные global/site quotas, очистка учебного пароля, theme switching.
- Индекс и справочник: 320 / 375 / 414 / 768 / 1280, offline full-text search.
- Умные/ручные группы: состав/счётчики, RAM и диапазон, И/ИЛИ/Unknown,
  установленная/библиотечная версия, создание/изменение/удаление и отмена.
- Вложенные папки: union без повторов, несколько уровней, смена родителя,
  исключение self/descendant из родителей, защита папки с детьми.
- Смена учебного эталона 130 → 131: пересчёт умных групп, неизменность ручной.
- Редакторы умной/ручной группы и папки на семи ширинах; дерево-навигация
  на 320/375/414/768; поиск по полному пути группы.

### Исправлено при проверке

- Добавлен статический content anchor для skip link до загрузки JS.
- Сокращены подписи боковой навигации: полные названия сохранены в aria-label.
- На 1024 px карточка ПК переставляется ниже таблицы: не обрезаются колонки.
- Длинное значение URL внутри input не считается overflow всей страницы:
  single-line input может штатно прокручивать собственный текст.
- Длинные пути подгрупп в общем поиске не расширяют dialog на узком экране.
- Позднее закрытие старого dialog не очищает уже заново открытый редактор.

## Визуальные ограничения проверки

Просмотрены реальные screenshots исходных приложений и новых макетов,
включая A на desktop viewport 1280 px. При доработке групп просмотрены дерево,
состав группы и редактор с live preview в светлой теме, а также тёмный экран групп.
Геометрия остальных ширин проверена автоматикой. Не заявляется ручная приёмка всех
комбинаций экранов, DPI и ОС, а также формальный результат «58/58» дизайн-гейтов.
Системные Segoe UI/Consolas выбраны сознательно ради офлайн Windows-прототипов.

Измеренные пары: ink/surface, muted/surface, muted/soft, accent/accent-soft,
on-accent/accent, good/good-soft, warn/warn-soft, error/surface. Это не полный
автоматический accessibility audit каждого hover/disabled/focus в каждом состоянии.

## Воспроизведение

Статические команды см. в `readme.md`. Для browser harness временно раздать **только
эту папку** через localhost HTTP и в её странице выполнить в DevTools:

```javascript
const { runChecks } = await import('/tools/browser-checks.mjs');
await runChecks({ paths: ['/mockups/sidebar.html'], includeDocs: false });
await runChecks({ paths: ['/mockups/tabs.html'], includeGroupScenarios: false });
```

Harness создаёт и удаляет проверочные iframe. Не предназначен для production
и не включается макетами автоматически. Точный адрес/порт временного сервера
не является настройкой Orkestr. Сами HTML открываются напрямую двойным щелчком.

## Не выполнялось

- Cargo build/tests исходных проектов повторно: прочитаны их прежние отчёты,
  но в них ничего не менялось; результаты не переназначаются Orkestr.
- Реальная аутентификация, DPAPI нового Orkestr, download verification,
  remote scan/deploy, durable scheduler, rate limiting — нет реализации для испытаний.
- Domain/workgroup acceptance на двух ПК, SCM install/reboot, recovery drill.
- Полный screen-reader/Windows High Contrast/DPI review, production security audit.

Следующие ворота перечислены в [testing.md](testing.md), а не считаются пройденными.
