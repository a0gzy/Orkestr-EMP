/* Offline interaction prototype. No network, real credentials or installer execution. */
"use strict";
(() => {
  const root = document.documentElement;
  const layout = root.dataset.layout;
  const app = document.getElementById("app");
  const modal = document.getElementById("modal");
  const media = matchMedia("(prefers-color-scheme: dark)");
  let theme = "system";
  try { theme = localStorage.getItem("orkestr-demo-theme") || "system"; } catch (_) { /* file storage may be unavailable */ }
  if (!["system", "light", "dark"].includes(theme)) theme = "system";
  const applyTheme = () => { root.dataset.mode = theme === "system" ? (media.matches ? "dark" : "light") : theme; };
  media.addEventListener("change", applyTheme);
  applyTheme();
  const esc = value => String(value ?? "").replace(/[&<>"']/g, c => ({"&":"&amp;", "<":"&lt;", ">":"&gt;", '"':"&quot;", "'":"&#39;"}[c]));
  const shapes = {
    pc: '<rect x="3" y="4" width="18" height="13" rx="2"/><path d="M8 21h8m-4-4v4"/>',
    box: '<path d="m12 3 9 5v9l-9 5-9-5V8zM3 8l9 5 9-5m-9 5v9M7.5 5.5l9 5"/>',
    queue: '<path d="M9 5h12M9 12h12M9 19h12M3 5h1M3 12h1M3 19h1"/>',
    sliders: '<path d="M4 7h4m4 0h8M4 17h10m4 0h2"/><circle cx="10" cy="7" r="2"/><circle cx="16" cy="17" r="2"/>',
    clock: '<circle cx="12" cy="12" r="9"/><path d="M12 7v5l3 2"/>',
    settings: '<path d="m9 3-1 3-3 1-2 5 2 5 3 1 1 3h6l1-3 3-1 2-5-2-5-3-1-1-3z"/><circle cx="12" cy="12" r="3"/>',
    search: '<circle cx="10" cy="10" r="6"/><path d="m15 15 6 6"/>',
    plus: '<path d="M12 4v16M4 12h16"/>',
    shield: '<path d="m12 3 8 3v6c0 5-8 10-8 10S4 17 4 12V6z"/><path d="m8 12 3 3 5-6"/>',
    check: '<path d="m5 12 4 4L19 6"/>',
    info: '<circle cx="12" cy="12" r="9"/><path d="M12 11v6m0-10v1"/>',
    arrow: '<path d="M4 12h16m-6-6 6 6-6 6"/>',
    key: '<circle cx="8" cy="8" r="5"/><path d="m12 12 9 9m-3-3 3-3m-6 0 3-3"/>',
    server: '<rect x="3" y="3" width="18" height="7" rx="2"/><rect x="3" y="14" width="18" height="7" rx="2"/><path d="M7 6h.1M7 17h.1"/>',
    book: '<path d="M12 5c-4-3-9-2-9-2v16s5-1 9 2c4-3 9-2 9-2V3s-5-1-9 2zm0 0v16"/>',
    groups: '<rect x="3" y="4" width="7" height="6" rx="1"/><rect x="14" y="4" width="7" height="6" rx="1"/><rect x="8.5" y="16" width="7" height="5" rx="1"/><path d="M6.5 10v3h11v-3M12 13v3"/>'
  };
  const icon = name => `<svg class="icon" viewBox="0 0 24 24" aria-hidden="true">${shapes[name] || shapes.info}</svg>`;
  const mark = '<svg class="brand-mark" viewBox="0 0 32 32" fill="none" aria-hidden="true"><path d="M4 23V9l7-4 7 4v14l-7 4-7-4Zm14-14 7-4 3 2v16l-7 4-3-2" stroke="currentColor" stroke-width="2.3"/><path d="m4 9 7 4 7-4m-7 4v14" stroke="currentColor" stroke-width="2.3"/></svg>';
  const brand = `<a class="brand" href="../index.html">${mark}Orkestr <small>EMP</small></a>`;
  const tag = (text, tone = "", glyph = "") => `<span class="tag ${tone}">${glyph ? icon(glyph) : ""}${esc(text)}</span>`;
  const notice = (text, tone = "") => `<div class="notice ${tone}">${icon("info")}<p>${esc(text)}</p></div>`;
  const devices = [
    {id:1, name:"TEST-PC-01", host:"test-pc-01.lab.test", source:"AD", site:"Основной офис", os:"Windows 11 Pro", ip:"192.0.2.11", state:"Скан завершён", profile:"Домен · LAB\\svc-emp", ram:"16 ГБ", cpu:"4 ядра", software:"Atlas Editor 2.3.0"},
    {id:2, name:"TEST-PC-02", host:"test-pc-02.lab.test", source:"AD", site:"Основной офис", os:"Windows 11 Pro", ip:"192.0.2.12", state:"Скан завершён", profile:"Домен · LAB\\svc-emp", ram:"32 ГБ", cpu:"8 ядер", software:"Atlas Editor 2.4.1"},
    {id:3, name:"TEST-PC-03", host:"test-pc-03.lab.test", source:"AD", site:"Основной офис", os:"Windows 10 Pro", ip:"192.0.2.13", state:"Частичный скан", profile:"Домен · LAB\\svc-emp", ram:"16 ГБ", cpu:"4 ядра", software:"Неизвестно: провайдер ПО недоступен"},
    {id:4, name:"TEST-SRV-01", host:"test-srv-01.lab.test", source:"AD", site:"Серверная", os:"Windows Server", ip:"192.0.2.21", state:"Скан завершён", profile:"Домен · LAB\\svc-emp", ram:"64 ГБ", cpu:"8 ядер", software:"Atlas Editor не обнаружен"},
    {id:5, name:"TEST-WG-01", host:"test-wg-01.lab.test", source:"Workgroup", site:"Основной офис", os:"Windows 11 Pro", ip:"192.0.2.31", state:"Скан завершён", profile:"Локальная · TEST-WG-01\\emp", ram:"16 ГБ", cpu:"4 ядра", software:"Atlas Editor 2.3.0"},
    {id:6, name:"TEST-WG-02", host:"test-wg-02.lab.test", source:"Workgroup", site:"Основной офис", os:"—", ip:"192.0.2.32", state:"Нет связи", profile:"Локальная · TEST-WG-02\\emp", ram:"—", cpu:"—", software:"Неизвестно: нет актуального скана"}
  ];
  const applications = [
    {id:1, name:"Atlas Editor", initials:"Ae", description:"Учебный редактор · не реальный продукт", version:"2.4.1", previous:"2.3.0", kind:"json", path:"/version", approved:false},
    {id:2, name:"Orbit Browser", initials:"Ob", description:"Учебный браузер · стабильный канал", version:"8.12.0", previous:"8.11.0", kind:"html", path:".release-version", approved:false},
    {id:3, name:"North Archive", initials:"Na", description:"Учебный архиватор · локальный пакет", version:"3.2.0", previous:"3.2.0", kind:"text", path:"", approved:true},
    {id:4, name:"Vector Reader", initials:"Vr", description:"Учебный просмотрщик · требуется проверка", version:"1.10.2", previous:"1.9.0", kind:"json", path:"/version", approved:false}
  ];
  if (layout === "sidebar") {
    // Deliberately synthetic Chrome versions, not a live vendor release claim.
    applications.push({id:5,name:"Google Chrome · демо",initials:"Ch",description:"Учебные версии, не текущие релизы Google",version:"130.0.0.0",previous:"129.0.0.0",kind:"json",path:"/version",approved:false});
    devices.push({id:7,name:"TEST-PC-04",host:"test-pc-04.lab.test",source:"AD",site:"Основной офис",os:"Windows 10 Pro",ip:"192.0.2.14",state:"Скан завершён",profile:"Домен · LAB\\svc-emp",ram:"4 ГБ",cpu:"2 ядра",software:"Atlas Editor 2.3.0"});
    const ram={1:16,2:32,3:8,4:64,5:16,6:null,7:4};
    const installed={1:{1:"2.3.0",5:"129.0.0.0"},2:{1:"2.4.1",5:"130.0.0.0"},3:null,4:{},5:{1:"2.3.0",5:"130.0.0.0"},6:null,7:{1:"2.3.0",5:"129.0.0.0"}};
    devices.forEach(d => { d.ramGB=ram[d.id]; d.ram=d.ramGB === null ? "—" : `${d.ramGB} ГБ`; d.installed=installed[d.id]; });
  }
  const state = {page:layout === "tabs" ? "library" : "computers", search:"", filter:"all", device:1, application:1, deviceTab:"overview", connectTab:"ad", activity:[], approvedPlan:false,
    policy:{global:2, site:1, install:2, jitter:120, check:true, download:false, auto:false}, drafts:{}};
  const pages = {
    computers:["Компьютеры", "Оборудование, установленное ПО и задания — в одной карточке.", "pc"],
    ...(layout === "sidebar" ? {groups:["Группы компьютеров", "Умные — по условиям. Ручные — по вашему выбору. Состав и установленное ПО видны сразу.", "groups"]} : {}),
    library:["Библиотека приложений", "От источника версии до проверенного пакета. Установка — отдельное решение.", "box"],
    jobs:["Очередь развёртывания", "Поэтапная передача и установка без одновременного старта всех ПК.", "queue"],
    policies:["Политики обновлений", "Раздельно задайте проверку, загрузку, одобрение и ограничения сети.", "sliders"],
    activity:["Журнал действий", "Только действия этой демонстрации. Это не журнал реального сервера.", "clock"],
    settings:["Настройки сервера", "Один сервер, общая база, несколько консолей. В первой версии — без агентов.", "settings"]
  };
  const groups = layout === "sidebar" ? window.OrkestrGroups.create({devices,applications,esc,icon,tag,notice,refresh:render,navigate:route,openDialog,closeDialog:() => modal.close(),log:text => log(text),showDevice:id => { state.device=id; state.deviceTab="software"; route("computers"); },showApplication:id => { state.application=id; route("library"); }}) : null;
  const log = text => { state.activity.unshift({time:new Date().toLocaleTimeString("ru-RU"), text}); };
  function route(page) {
    state.page = pages[page] ? page : "computers"; state.search = ""; state.filter = "all";
    history.replaceState(null, "", `#${state.page}`); render();
  }
  function navButton(key, extra = "") {
    const shortLabels = {computers:"Компьютеры",groups:"Группы",library:"Библиотека",jobs:"Развёртывания",policies:"Политики",activity:"Журнал",settings:"Настройки"};
    return `<button class="nav-link" data-page="${key}" aria-label="${esc(pages[key][0])}" ${state.page === key ? 'aria-current="page"' : ""}>${icon(pages[key][2])}${esc(layout === "sidebar" ? shortLabels[key] : pages[key][0])}${extra}</button>`;
  }
  function topbar() {
    return `<header class="topbar">${layout === "tabs" ? `<div class="top-brand">${brand}<div class="workspace-tabs" aria-label="Рабочие разделы"><button data-page="computers" ${state.page === "computers" ? 'aria-current="page"' : ""}>Инвентаризация</button><button data-page="library" ${["library","jobs","policies"].includes(state.page) ? 'aria-current="page"' : ""}>Развёртывание</button></div></div>` : '<span class="crumb">Рабочее пространство / <strong>Учебная сеть</strong></span>'}
      <button class="search-button" data-action="search" aria-label="Общий поиск, Ctrl K">${icon("search")}<span class="search-label">Найти в Orkestr</span><kbd>Ctrl K</kbd></button>
      <label class="theme-control"><span class="small">Тема</span><select id="theme" aria-label="Тема интерфейса"><option value="system">Системная</option><option value="light">Светлая</option><option value="dark">Тёмная</option></select></label>
      <a class="btn quiet" href="../docs/index.html" aria-label="Справочник">${icon("book")}<span>Справка</span></a>
    </header>`;
  }
  function render() {
    const navigation = Object.keys(pages).map(k => navButton(k, k === "computers" ? `<span class="count">${devices.length}</span>` : "")).join("");
    const side = `<aside class="sidebar">${brand}<div class="environment">${icon("server")}<div>Учебная сеть<small>Нет подключения</small></div></div><div class="sidebar-scroll"><nav class="nav-stack" aria-label="Основные разделы">${navigation}</nav>${state.page === "groups" ? groups.sidebar() : ""}</div><div class="sidebar-bottom"><a class="btn quiet" href="tabs.html">Сравнить: макет B ${icon("arrow")}</a><p class="small muted">Концепция A<br>Боковая навигация</p></div></aside>`;
    let tabNav = "";
    if (layout === "tabs") tabNav = `<nav class="section-nav" aria-label="Разделы рабочего пространства">${Object.keys(pages).filter(k => k !== "computers").map(k => navButton(k)).join("")}</nav>`;
    app.innerHTML = `<div class="layout">${layout === "sidebar" ? side : ""}<div class="main">${topbar()}${tabNav}<div class="demo-banner">Интерактивный макет ${layout === "sidebar" ? "A" : "B"} · Все компьютеры, версии и статусы — учебные. Сетевых действий нет.</div><main class="page"><div class="page-head"><div><h1>${pages[state.page][0]}</h1><p>${pages[state.page][1]}</p></div><div class="actions"><button data-action="credentials">${icon("key")}Учётные данные</button><button class="primary" data-action="connect">${icon("plus")}Подключить ПК</button></div></div><div id="content" tabindex="-1"></div><footer class="foot"><span>Orkestr EMP · ${layout === "sidebar" ? "Концепция A / Workbench" : "Концепция B / Split Studio"} · Без агентов</span><div><a href="${layout === "sidebar" ? "tabs.html" : "sidebar.html"}">Макет ${layout === "sidebar" ? "B" : "A"}</a><a href="../index.html">К сравнению</a><a href="../docs/index.html">Инструкция</a></div></footer></main></div></div>`;
    document.getElementById("theme").value = theme;
    renderContent();
  }
  function searchField(placeholder) { return `<label class="small"><span class="muted">Поиск</span><input id="list-search" type="search" value="${esc(state.search)}" placeholder="${esc(placeholder)}"></label>`; }
  function renderContent() {
    const views = {computers:computerView, groups:() => groups.view(), library:libraryView, jobs:queueView, policies:policyView, activity:activityView, settings:settingsView};
    document.getElementById("content").innerHTML = views[state.page]();
  }
  function computerView() {
    const rows = devices.filter(d => (state.filter === "all" || d.source === state.filter) && `${d.name} ${d.host} ${d.os} ${d.ip}`.toLowerCase().includes(state.search.toLowerCase()));
    const d = devices.find(d => d.id === state.device);
    const software = layout === "sidebar" && d.installed ? (Object.entries(d.installed).map(([id,version]) => `<p>${esc(applications.find(a => a.id === Number(id))?.name || "Приложение")} <span class="mono">${esc(version)}</span></p>`).join("") || '<p>Приложения из учебной библиотеки не обнаружены.</p>') : `<p>${esc(d.software)}</p>`;
    const detail = state.deviceTab === "software" ? `<h3>Установленное ПО</h3>${software}${notice("Отсутствие сведений при ошибке скана — неизвестное состояние, не отсутствие приложения.")}<button data-page="library">Открыть библиотеку ${icon("arrow")}</button>`
      : state.deviceTab === "jobs" ? `${notice("Реальных заданий нет. Очередь содержит только расчётный пример.")}<button data-page="jobs">Просмотреть очередь ${icon("arrow")}</button>`
      : `<dl><div><dt>Источник</dt><dd>${d.source === "AD" ? "Active Directory" : "Вне домена"}</dd></div><div><dt>Площадка</dt><dd>${d.site}</dd></div><div><dt>ОС</dt><dd>${d.os}</dd></div><div><dt>Оперативная память</dt><dd>${d.ram}</dd></div><div><dt>Процессор</dt><dd>${d.cpu}</dd></div><div><dt>Транспорт</dt><dd>WinRM / CIM</dd></div></dl><div><h3>Профиль подключения</h3><p class="small muted">${esc(d.profile)}</p></div>${notice("Проверка доступа и расшифровка пароля выполняются сервером, а не клиентской консолью.")}<button data-action="credentials">${icon("shield")}Изменить профиль</button>`;
    return `<div class="summary"><div><strong>${devices.length}</strong><span>учебных ПК</span></div><div><strong>${devices.filter(d => d.source === "AD").length}</strong><span>из домена</span></div><div><strong>${devices.filter(d => d.source !== "AD").length}</strong><span>вне домена</span></div><div><strong>—</strong><span>реальные подключения</span></div></div><div class="workspace"><section class="panel"><div class="panel-head"><div class="filters" aria-label="Источник компьютеров">${[["all","Все ПК"],["AD","Домен"],["Workgroup","Вне домена"]].map(([v,t]) => `<button class="filter" data-filter="${v}" aria-pressed="${state.filter === v}">${t}</button>`).join("")}</div>${searchField("Имя, IP или ОС")}</div><div id="list-results">${computerTable(rows)}</div></section><aside class="panel" aria-label="Карточка компьютера"><div class="panel-body"><div class="detail-title"><div class="device-avatar">${icon("pc")}</div><div><h2>${esc(d.name)}</h2><p>Учебная карточка</p></div></div></div><div class="subtabs" aria-label="Разделы карточки">${[["overview","Обзор"],["software","ПО"],["jobs","Задания"]].map(([v,t]) => `<button data-device-tab="${v}" aria-pressed="${state.deviceTab === v}">${t}</button>`).join("")}</div><div class="panel-body">${detail}</div></aside></div>`;
  }
  function computerTable(rows) {
    if (!rows.length) return '<div class="empty"><strong>Компьютеры не найдены</strong>Измените поиск или источник.</div>';
    return `<table><thead><tr><th>Компьютер</th><th>Операционная система</th><th>Источник</th><th>Состояние примера</th></tr></thead><tbody>${rows.map(d => `<tr class="${d.id === state.device ? "selected" : ""}"><td data-label="Компьютер"><button class="row-link" data-device="${d.id}">${esc(d.name)}</button><span class="muted mono">${d.ip}</span></td><td data-label="ОС">${d.os}</td><td data-label="Источник">${d.source === "AD" ? "Домен" : "Вне домена"}</td><td data-label="Состояние">${tag(d.state, d.state === "Скан завершён" ? "good" : "warn", d.state === "Скан завершён" ? "check" : "info")}</td></tr>`).join("")}</tbody></table><div class="table-bottom"><span>Показано ${rows.length} из ${devices.length}</span><span>Выберите имя для просмотра</span></div>`;
  }
  function libraryView() {
    const a = applications.find(a => a.id === state.application);
    const rows = applications.filter(a => a.name.toLowerCase().includes(state.search.toLowerCase()));
    return `<div class="summary"><div><strong>${applications.length}</strong><span>учебных приложений</span></div><div><strong>${applications.filter(a => !a.approved).length}</strong><span>ожидают одобрения</span></div><div><strong>API / HTML</strong><span>источники версии</span></div></div><div class="workspace library-workspace"><section class="panel"><div class="panel-head"><h2>Приложения</h2><button data-action="new-app">${icon("plus")}Добавить</button>${searchField("Название приложения")}</div><div id="list-results">${libraryTable(rows)}</div><div class="panel-body">${notice("Приложения и номера версий вымышлены. Установщики в макет не включены.")}</div></section><section class="panel"><div class="panel-head"><div class="app-heading"><span class="app-monogram">${esc(a.initials)}</span><div><h2>${esc(a.name)}</h2><span class="small muted">stable · x64 · учебный источник</span></div></div>${tag(a.approved ? "Одобрено в демо" : "Нужно одобрение", a.approved ? "good" : "warn", a.approved ? "check" : "clock")}</div><div class="panel-body">${sourceForm(a)}<div class="actions"><button data-action="approve" ${a.approved ? "disabled" : ""}>${icon("shield")}Одобрить версию</button><button data-page="jobs">План развёртывания ${icon("arrow")}</button></div></div></section></div>`;
  }
  function libraryTable(rows) {
    return rows.length ? `<table><thead><tr><th>Приложение</th><th>Версия кандидата</th><th>Источник</th></tr></thead><tbody>${rows.map(a => `<tr class="${a.id === state.application ? "selected" : ""}"><td><button class="row-link" data-application="${a.id}">${esc(a.name)}</button><span class="muted">${a.approved ? "Одобрено в демо" : "Ожидает одобрения"}</span></td><td data-label="Кандидат"><span class="mono">${a.version}</span></td><td data-label="Источник">${({json:"JSON API",html:"HTML",text:"Текст"})[a.kind]}</td></tr>`).join("")}</tbody></table>` : '<div class="empty"><strong>Приложения не найдены</strong>Измените строку поиска.</div>';
  }
  function sourceForm(a) {
    const draft = state.drafts[a.id] || {};
    const kind = draft.kind || a.kind;
    const value = (key, fallback) => {
      const defaultValue=key === "pattern" ? "(\\d+(?:\\.\\d+){1,3})" : key === "downloadPattern" ? "app-(\\d+(?:\\.\\d+){1,3})-x64\\.msi$" : fallback;
      return esc(draft[key] ?? defaultValue);
    };
    const downloadKind = draft.downloadKind || "template";
    return `<form id="source-form" class="form-grid"><div class="fields-two"><label>Тип источника<select name="kind" id="source-kind">${[["json","JSON API"],["html","HTML / CSS selector"],["text","Текст / Regex"]].map(([v,t]) => `<option value="${v}" ${v === kind ? "selected" : ""}>${t}</option>`).join("")}</select></label><label>Кандидат<input value="${a.version}" readonly aria-label="Учебная версия кандидата"></label></div><label>URL источника<input name="url" type="url" value="${value("url", "https://releases.example.test/" + (kind === "json" ? "api/latest" : "downloads"))}" required><small>В макете запрос не отправляется. Используется локальный пример.</small></label>${kind !== "text" ? `<label>${kind === "json" ? "JSON Pointer" : "CSS selector"}<input name="extractor" value="${value("extractor", kind === "json" ? "/version" : ".release-version")}" required></label>` : ""}${kind === "html" ? `<label>Атрибут версии · необязательный<input name="attribute" value="${value("attribute", "")}" placeholder="Пусто — текст элемента"><small>В примере также есть атрибут data-version.</small></label>` : ""}<label>Regex версии · группа 1<input class="mono" name="pattern" value="${value("pattern", "(\\d+\\.\\d+(?:\\.\\d+)?)")}"><small>Например: (\d+\.\d+(?:\.\d+)?) · необязательный.</small></label><label>Как получить установщик<select name="downloadKind" id="download-kind"><option value="template" ${downloadKind === "template" ? "selected" : ""}>URL-шаблон с версией</option><option value="link" ${downloadKind === "link" ? "selected" : ""}>Ссылка из HTML + Regex</option></select></label><label>${downloadKind === "link" ? "URL страницы загрузки" : "URL установщика"}<input name="download" value="${value("download", "https://downloads.example.test/app/{version}/setup-x64.msi")}"><small>${downloadKind === "link" ? "Href разрешается относительно этой страницы. Запросов в макете нет." : "Переменная {version} заменяется найденным номером."}</small></label>${downloadKind === "link" ? `<label>CSS selector ссылки<input name="downloadSelector" value="${value("downloadSelector", "a.release-download")}"></label><label>Regex пути файла · версия в группе 1<input class="mono" name="downloadPattern" value="${value("downloadPattern", "app-(\\d+\\.\\d+(?:\\.\\d+)?)-x64\\.msi$")}"><small>Номер в ссылке должен совпасть с версией источника.</small></label><details><summary>Учебная страница загрузки</summary><pre>${esc(downloadFixture(a.version))}</pre></details>` : ""}<details><summary>Локальный ответ для проверки</summary><pre id="source-fixture">${esc(fixture(kind, a.version))}</pre></details><div class="actions"><button type="button" data-action="test-source">${icon("search")}Проверить пример</button><button type="submit" class="primary">Сохранить в демо</button></div><div id="source-result" class="result" role="status">Проверка и сохранение не разрешают установку.</div></form>`;
  }
  function fixture(kind, version) { return kind === "json" ? JSON.stringify({version, channel:"stable"}, null, 2) : kind === "html" ? `<span class="release-version" data-version="${version}">Version ${version}</span>` : `Current release: ${version} (stable)`; }
  function downloadFixture(version) { return `<a class="release-download" href="/files/app-${version}-x64.msi">Download</a>`; }
  function queueView() {
    const a = applications.find(a => a.id === state.application);
    const ready = a.approved;
    const limit = Math.min(state.policy.global, state.policy.site);
    return `<div class="workspace"><section class="panel"><div class="panel-head"><div><h2>${esc(a.name)} · ${a.version}</h2><p class="small muted">Расчётный пример · не запущено</p></div>${tag(ready ? "Можно планировать" : "Ожидает одобрения", ready ? "good" : "warn", ready ? "check" : "clock")}</div><div class="panel-body">${notice("Это проекция очереди, а не наблюдение за сетью. Одобрение и план сохраняются только до перезагрузки страницы.")}</div><table><thead><tr><th>Цель</th><th>Площадка</th><th>Проекция / причина</th></tr></thead><tbody>${devices.filter(d => d.id !== 4).map((d,i) => `<tr><td><strong>${d.name}</strong></td><td data-label="Площадка">${d.site}</td><td data-label="Причина">${tag(!ready ? "Нужно одобрение" : d.id === 6 ? "Ожидание связи" : i < limit ? "Слот передачи" : "Ожидание квоты", !ready || d.id === 6 ? "warn" : "", !ready ? "shield" : "clock")}</td></tr>`).join("")}</tbody></table><div class="panel-body"><div class="actions"><button class="primary" data-action="plan" ${!ready ? "disabled" : ""}>${icon("queue")}Сформировать план</button><button data-action="approve" ${ready ? "disabled" : ""}>Одобрить версию</button></div><div id="plan-result" class="result" role="status">${state.approvedPlan ? "Учебный план сформирован. Реальных задач не создано." : "Сервер не подключён: реальный запуск недоступен."}</div></div></section><aside class="panel"><div class="panel-head"><h2>Бюджет нагрузки</h2></div><div class="panel-body"><dl><div><dt>Передач глобально</dt><dd>${state.policy.global}</dd></div><div><dt>Передач на площадку</dt><dd>${state.policy.site}</dd></div><div><dt>Установок</dt><dd>${state.policy.install}</dd></div><div><dt>Разброс старта</dt><dd>до ${state.policy.jitter} с</dd></div><div><dt>Задач на ПК</dt><dd>1</dd></div></dl>${notice("Лимит Мбит/с требует нового транспортного адаптера. Число передач — не гарантия скорости.", "warn")}<button data-page="policies">Настроить ограничения ${icon("arrow")}</button></div></aside></div>`;
  }
  function policyView() {
    const p = state.policy;
    return `<div class="workspace library-workspace"><section class="panel"><div class="panel-head"><h2>Обновления · учебная политика</h2>${tag("Изменения только в памяти")}</div><form id="policy-form" class="panel-body"><div class="step-list"><div class="step-row"><span class="step-number">1</span><div><h3>Найти новую версию</h3><label class="check"><input name="check" type="checkbox" ${p.check ? "checked" : ""}>Автопроверка источников</label><p>Предлагаемый интервал — 6 часов с разбросом старта.</p></div></div><div class="step-row"><span class="step-number">2</span><div><h3>Скачать на сервер</h3><label class="check"><input name="download" type="checkbox" ${p.download ? "checked" : ""}>Автозагрузка в карантин</label><p>Одна серверная загрузка; не означает установку на ПК.</p></div></div><div class="step-row"><span class="step-number">3</span><div><h3>Одобрить и развернуть</h3><label class="check"><input name="auto" type="checkbox" ${p.auto ? "checked" : ""}>Автополитика вместо ручного одобрения</label><p>Только после проверок, успешного пилота и в разрешённое окно.</p></div></div></div>${notice("По умолчанию версия ждёт одобрения администратора. Для production-автополитики нужно выбрать пилот, окно, доверенного издателя и порог ошибок.", "warn")}<fieldset class="form-section"><legend>Ограничения общей очереди</legend><div class="fields-two"><label>Передач глобально<input name="global" type="number" min="1" max="32" value="${p.global}" required></label><label>Передач на площадку<input name="site" type="number" min="1" max="32" value="${p.site}" required></label><label>Одновременных установок<input name="install" type="number" min="1" max="32" value="${p.install}" required></label><label>Разброс старта, секунд<input name="jitter" type="number" min="0" max="3600" value="${p.jitter}" required></label></div></fieldset><div class="actions"><button type="submit" class="primary">Сохранить в демо</button><button type="button" data-page="jobs">Предпросмотр очереди</button></div><div class="result" id="policy-result" role="status">Новые лимиты меняют выдачу задач, а не прерывают текущие установки.</div></form></section><aside class="panel"><div class="panel-head"><h2>Как это будет работать</h2></div><div class="panel-body"><h3>Без шквала загрузок</h3><p class="muted">Общие квоты действуют на все задания, включая повторы. Сервер сначала передаёт проверенный пакет, затем запускает установку.</p><h3>Строго по одному</h3><p class="muted">Передачи = 1, установки = 1 и запрет перекрытия целей в полной версии.</p><h3>Сначала проверить</h3><p class="muted">Пилот, выдержка, окно обслуживания и остановка волны при ошибках. Это не только случайная задержка старта.</p><a href="../docs/index.html#updates">Открыть инструкцию</a></div></aside></div>`;
  }
  function activityView() {
    return `<section class="panel"><div class="panel-head"><h2>Действия в макете</h2>${tag("В памяти браузера")}</div><div class="panel-body">${state.activity.length ? state.activity.map(x => `<div class="log-row"><time class="mono muted">${esc(x.time)}</time><p>${esc(x.text)}</p></div>`).join("") : '<div class="empty"><strong>Действий ещё нет</strong>Проверьте источник версии или измените учебную политику.</div>'}${notice("В готовом приложении серверный аудит хранит автора, цель, revision и результат. Пароли и токены туда не попадают.")}</div></section>`;
  }
  function settingsView() {
    return `<div class="workspace"><section class="panel"><div class="panel-head"><h2>Схема установки</h2>${tag("Проектируется")}</div><div class="panel-body"><div class="step-list"><div class="step-row"><span class="step-number">1</span><div><h3>Серверная Windows-служба</h3><p>Общая SQLite, расписания, секреты и библиотека; работает без консоли.</p></div></div><div class="step-row"><span class="step-number">2</span><div><h3>Несколько консолей по HTTPS</h3><p>Персональная авторизация и роли. Клиенты не открывают базу через SMB.</p></div></div><div class="step-row"><span class="step-number">3</span><div><h3>Компьютеры через WinRM / CIM</h3><p>Доменные и недоменные подключения, без постоянного агента в V1.</p></div></div></div><label>Пример адреса сервера<input readonly value="https://emp-server.lab.test:7443"></label>${notice("Подключение в HTML отсутствует. Здесь нельзя настраивать реальную службу или проверять TLS.")}<div class="actions"><button data-action="credentials">${icon("key")}Учётные данные</button><button data-action="connect">${icon("pc")}Подключение ПК</button></div></div></section><aside class="panel"><div class="panel-head"><h2>Хранение секретов</h2></div><div class="panel-body"><p>DPAPI CurrentUser под постоянной учётной записью службы.</p>${notice("Смена сервера или identity службы требует повторного ввода паролей. DPAPI не заменяет резервную копию.", "warn")}<a href="../docs/index.html#credentials">Порядок восстановления</a></div></aside></div>`;
  }
  function openDialog(title, subtitle, body, foot = '<button data-action="close">Закрыть</button>') {
    modal.classList.remove("group-editor-dialog");
    modal.innerHTML = `<header class="modal-head"><h2 id="modal-title">${esc(title)}</h2><p>${esc(subtitle)}</p></header>${body}<footer class="modal-foot">${foot}</footer>`;
    if (!modal.open) modal.showModal();
    const first = modal.querySelector("input,select,textarea,button");
    first?.focus({preventScroll:true});
  }
  function connectDialog(tab = state.connectTab) {
    state.connectTab = tab;
    const body = tab === "ad" ? `<label>Домен или контроллер<input name="domain" placeholder="dc01.lab.test" required></label><label>Область поиска OU / DN<input name="ou" placeholder="OU=Workstations,DC=lab,DC=test"></label><label>Профиль чтения AD<select name="profile"><option>Учётная запись службы сервера</option><option>LAB · Чтение каталога (учебный)</option></select></label><label class="check"><input type="checkbox" checked>Включать вложенные OU</label><label class="check"><input type="checkbox">Импортировать отключённые объекты</label>`
      : tab === "manual" ? `<label>Имена компьютеров<textarea name="hosts" placeholder="test-wg-01.lab.test&#10;test-pc-01.lab.test" required></textarea><small>По одному DNS-имени в строке. Проверка сертификата выполняется по имени.</small></label><label>Тип подключения<select name="connection"><option>Вне домена · WinRM HTTPS</option><option>Домен · Kerberos / WinRM</option></select></label><button type="button" data-action="credentials">${icon("key")}Открыть учётные данные</button>`
      : `<label>Учебный CSV<textarea name="csv" class="mono" required>hostname,source,site\ntest-pc-01.lab.test,AD,Основной офис\ntest-wg-01.lab.test,Workgroup,Основной офис</textarea><small>В полном приложении можно выбрать файл и сопоставить колонки.</small></label>`;
    openDialog("Подключить компьютеры", "Отдельное окно интеграции · действия выполняются на сервере, не на консоли.", `<div class="subtabs" aria-label="Источник компьютеров">${[["ad","Active Directory"],["manual","Вручную"],["csv","CSV"]].map(([v,t]) => `<button data-connect-tab="${v}" aria-pressed="${v === tab}">${t}</button>`).join("")}</div><form id="connect-form" class="modal-body">${body}${notice("Учебная форма: не вводите реальные реквизиты. Preview ниже не отправляет запросы в сеть.")}<button class="primary" type="submit">Показать пример preview</button><div id="connect-result" class="result" role="status">Импорт не удаляет отсутствующие ПК и не меняет настройки Windows.</div></form>`, '<button data-action="close">Закрыть</button>');
  }
  function credentialsDialog() {
    openDialog("Учётные данные", "Профиль подключения · пароль хранится только на сервере под identity его службы.", `<form id="credential-form" class="modal-body"><div class="notice warn">${icon("shield")}<p>Это макет, DPAPI здесь нет. Не вводите настоящий пароль. Учебное значение не сохраняется.</p></div><label>Назначение профиля<select name="scope"><option>Подключение к ПК / сканирование и установка</option><option>Чтение Active Directory</option></select></label><div class="fields-two"><label>Название профиля<input name="label" placeholder="Основной офис · локальная запись" required></label><label>Тип учётной записи<select name="kind"><option>Доменная</option><option>Локальная на ПК</option></select></label></div><label>Имя пользователя<input name="username" placeholder="LAB\svc-emp или ПК\emp" autocomplete="off" required></label><label>Учебное значение пароля<input type="password" name="password" autocomplete="new-password" required aria-describedby="password-hint"><small id="password-hint">Не будет записано в localStorage, журнал или файл. Очищается при закрытии.</small></label>${notice("В приложении: DPAPI CurrentUser + ACL. После обычного рестарта той же службы пароль доступен; после смены identity потребуется повторный ввод.")}<button class="primary" type="submit">Показать сценарий</button><div id="credential-result" class="result" role="status">Сначала проверяется доступ именно с сервера.</div></form>`);
  }
  function approvalDialog() {
    const a = applications.find(a => a.id === state.application);
    if (a.approved) return;
    openDialog("Одобрить учебную версию", `${a.name} ${a.version} · stable / x64`, `<form id="approval-form" class="modal-body">${notice("В полной версии одобрение закрепляет package revision и SHA-256. В макете реального файла и проверки подписи нет.", "warn")}<label class="check"><input name="ack" type="checkbox" required>Понимаю: это только демонстрация одобрения</label><button type="submit" class="primary">Одобрить в демо</button><p class="helper">Это не запускает установку. Планирование выполняется отдельно.</p></form>`);
  }
  function searchDialog() {
    openDialog("Найти в Orkestr", "Разделы и учебные объекты · Ctrl K / Esc · Tab или стрелки, Enter для выбора.", '<div class="modal-body"><label>Поиск<input id="global-search" type="search" placeholder="Компьютер, приложение или раздел" autofocus></label><div id="global-results" class="search-results"></div></div>');
    globalResults("");
  }
  function globalResults(query) {
    const items = [...Object.keys(pages).map(k => ({page:k, label:pages[k][0]})), ...(groups ? groups.searchItems().map(g => ({page:"groups",group:g.id,label:g.name})) : []), ...devices.map(d => ({page:"computers", device:d.id, label:d.name})), ...applications.map(a => ({page:"library", application:a.id, label:a.name}))];
    const list = items.filter(x => x.label.toLowerCase().includes(query.toLowerCase()));
    document.getElementById("global-results").innerHTML = list.map(x => `<button data-search-page="${x.page}" ${x.device ? `data-search-device="${x.device}"` : ""} ${x.application ? `data-search-application="${x.application}"` : ""} ${x.group ? `data-search-group="${esc(x.group)}"` : ""}>${icon(pages[x.page][2])}<span class="search-result-label">${esc(x.label)}</span></button>`).join("") || '<p class="muted">Совпадений нет. Измените запрос.</p>';
  }
  function draftFromForm() {
    const f = document.getElementById("source-form");
    if (!f) return null;
    const draft = Object.fromEntries(new FormData(f));
    state.drafts[state.application] = draft;
    return draft;
  }
  function result(id, text, status = "") {
    const el = document.getElementById(id); if (!el) return;
    el.textContent = text; el.dataset.state = status;
  }
  async function testSource(button) {
    const draft = draftFromForm();
    const a = applications.find(a => a.id === state.application);
    if (!draft) return;
    button.disabled = true; button.dataset.state = "loading"; button.setAttribute("aria-busy", "true");
    result("source-result", "Проверка локального примера…");
    try {
      const sourceURL = new URL(draft.url);
      if (sourceURL.protocol !== "https:") throw new Error("Нужен HTTPS URL источника.");
      let text = fixture(draft.kind, a.version);
      if (draft.kind === "json") {
        if (draft.extractor !== "" && !draft.extractor.startsWith("/")) throw new Error("JSON Pointer должен начинаться с /.");
        let value = JSON.parse(text);
        if (draft.extractor) for (const key of draft.extractor.slice(1).split("/")) value = value?.[key.replace(/~1/g,"/").replace(/~0/g,"~")];
        if (typeof value !== "string") throw new Error("JSON Pointer не выбрал строку версии. Попробуйте /version.");
        text = value;
      } else if (draft.kind === "html") {
        // DOMParser document is inert and is never inserted into the live document.
        const matches = new DOMParser().parseFromString(text, "text/html").querySelectorAll(draft.extractor);
        if (matches.length !== 1) throw new Error(`CSS selector нашёл ${matches.length} элементов. Нужен ровно один.`);
        text = draft.attribute ? matches[0].getAttribute(draft.attribute) : matches[0].textContent;
        if (text === null) throw new Error("У элемента нет указанного атрибута версии.");
      }
      const version = draft.pattern ? await boundedRegex(draft.pattern, text) : text.trim().replace(/^v/, "");
      if (!/^\d+(?:\.\d+){1,3}$/.test(version)) throw new Error("Не получен однозначный номер версии вида 2.4.1.");
      let url = draft.download.replaceAll("{version}", encodeURIComponent(version));
      if (draft.downloadKind === "link") {
        const matches = new DOMParser().parseFromString(downloadFixture(a.version), "text/html").querySelectorAll(draft.downloadSelector);
        if (matches.length !== 1 || !matches[0].getAttribute("href")) throw new Error("Селектор загрузки должен выбрать ровно одну ссылку с href.");
        url = new URL(matches[0].getAttribute("href"), draft.download).href;
        const fileVersion = await boundedRegex(draft.downloadPattern, new URL(url).pathname);
        if (fileVersion !== version) throw new Error("Версия в ссылке установщика не совпала с источником.");
      }
      if (new URL(url).protocol !== "https:") throw new Error("URL установщика должен использовать HTTPS.");
      result("source-result", `Локальный пример: версия ${version}. URL: ${url}. Запросов и загрузки не было.`, "success");
      log(`Проверен локальный пример источника ${a.name}. Сети не было.`);
    } catch (e) { result("source-result", `Проверка не пройдена: ${e.message}`, "error"); }
    finally { button.disabled = false; delete button.dataset.state; button.removeAttribute("aria-busy"); }
  }
  function boundedRegex(pattern, text) {
    if (pattern.length > 256) return Promise.reject(new Error("Regex длиннее 256 символов."));
    // Isolate the browser demonstration engine; Rust backend must use bounded Rust regex.
    return new Promise((resolve, reject) => {
      const code = 'onmessage = e => { try { const m = new RegExp(e.data.pattern).exec(e.data.text); postMessage({value:m ? (m[1] || m[0]) : null}); } catch(x) { postMessage({error:x.message}); } };';
      const url = URL.createObjectURL(new Blob([code], {type:"text/javascript"}));
      let worker;
      try { worker = new Worker(url); } catch (e) { URL.revokeObjectURL(url); reject(new Error("Браузер не разрешил изолированную regex-проверку.")); return; }
      const finish = () => { clearTimeout(timeout); worker.terminate(); URL.revokeObjectURL(url); };
      const timeout = setTimeout(() => { finish(); reject(new Error("Regex превысил время проверки.")); }, 800);
      worker.onmessage = event => { finish(); const r = event.data; r.error ? reject(new Error(r.error)) : r.value ? resolve(r.value) : reject(new Error("Regex не нашёл версию.")); };
      worker.onerror = () => { finish(); reject(new Error("Не удалось выполнить изолированную regex-проверку.")); };
      worker.postMessage({pattern, text:text.slice(0,2048)});
    });
  }
  document.addEventListener("click", e => {
    const button = e.target.closest("button"); if (!button || button.disabled) return;
    const d = button.dataset;
    if (groups?.handleClick(button)) return;
    if (d.page) { route(d.page); return; }
    if (d.searchPage) { if (d.searchDevice) state.device = Number(d.searchDevice); if (d.searchApplication) state.application = Number(d.searchApplication); if (d.searchGroup) groups.select(d.searchGroup); modal.close(); route(d.searchPage); return; }
    if (d.device) { state.device = Number(d.device); renderContent(); return; }
    if (d.application) { draftFromForm(); state.application = Number(d.application); renderContent(); return; }
    if (d.filter) { state.filter = d.filter; renderContent(); return; }
    if (d.deviceTab) { state.deviceTab = d.deviceTab; renderContent(); return; }
    if (d.connectTab) { connectDialog(d.connectTab); return; }
    if (!d.action) return;
    switch (d.action) {
      case "connect": connectDialog(); break;
      case "credentials": credentialsDialog(); break;
      case "search": searchDialog(); break;
      case "close": modal.close(); break;
      case "test-source": void testSource(button); break;
      case "approve": approvalDialog(); break;
      case "plan": state.approvedPlan = true; log("Учебный план сформирован. Установка не запускалась."); result("plan-result", "Учебный план сформирован. Никаких задач на сервере или ПК не создано.", "success"); break;
      case "new-app": openDialog("Новое приложение", "Учебный редактор · после добавления настройте источник и пакет.", '<form id="new-app-form" class="modal-body"><label>Название<input name="name" maxlength="60" required placeholder="Внутреннее приложение"></label><button class="primary" type="submit">Добавить в демо</button></form>'); break;
    }
  });
  document.addEventListener("change", e => {
    if (groups?.handleChange(e)) return;
    if (e.target.id === "theme") { theme = e.target.value; try { localStorage.setItem("orkestr-demo-theme", theme); } catch (_) {} applyTheme(); }
    if (e.target.id === "source-kind") {
      const draft = draftFromForm(); draft.extractor = draft.kind === "html" ? ".release-version" : "/version";
      renderContent();
    }
    if (e.target.id === "download-kind") {
      const draft = draftFromForm();
      draft.download = draft.downloadKind === "link" ? "https://downloads.example.test/releases" : "https://downloads.example.test/app/{version}/setup-x64.msi";
      renderContent();
    }
  });
  document.addEventListener("input", e => {
    if (groups?.handleInput(e)) return;
    if (e.target.id === "global-search") globalResults(e.target.value);
    if (e.target.id === "list-search") {
      state.search = e.target.value;
      const rows = state.page === "computers" ? devices.filter(d => (state.filter === "all" || d.source === state.filter) && `${d.name} ${d.host} ${d.os} ${d.ip}`.toLowerCase().includes(state.search.toLowerCase())) : applications.filter(a => a.name.toLowerCase().includes(state.search.toLowerCase()));
      document.getElementById("list-results").innerHTML = state.page === "computers" ? computerTable(rows) : libraryTable(rows);
    }
  });
  document.addEventListener("submit", e => {
    e.preventDefault();
    const form = e.target; const data = new FormData(form);
    if (form.id === "group-form") { groups.save(form); return; }
    if (form.id === "source-form") {
      const previous = JSON.stringify(state.drafts[state.application] || {});
      const draft = draftFromForm();
      const a = applications.find(a => a.id === state.application); a.kind = draft.kind;
      if (previous !== JSON.stringify(draft) || a.approved) { a.approved = false; state.approvedPlan = false; }
      log(`Изменён учебный источник ${a.name}. Одобрение сброшено.`);
      renderContent(); result("source-result", "Настройки в памяти демо. Одобрение сброшено; установки нет.", "success");
    } else if (form.id === "policy-form") {
      const global = Number(data.get("global")), site = Number(data.get("site"));
      if (site > global) { result("policy-result", "Лимит площадки не должен превышать глобальный. Уменьшите значение.", "error"); return; }
      state.policy = {global, site, install:Number(data.get("install")), jitter:Number(data.get("jitter")), check:data.has("check"), download:data.has("download"), auto:data.has("auto")};
      log("Учебная политика изменена. Production-автоматизация не включалась.");
      result("policy-result", "Политика сохранена только в демо. Реальных заданий и загрузок нет.", "success");
    } else if (form.id === "credential-form") {
      form.querySelector('[name="password"]').value = "";
      result("credential-result", "Сценарий: сервер проверит доступ и сохранит DPAPI blob. В HTML пароль только очищен — не сохранён и не зашифрован.");
    } else if (form.id === "connect-form") {
      result("connect-result", "Учебный preview: здесь появятся найденные ПК, дубликаты и ошибки. Реальный запрос не выполнялся; ничего не импортировано.");
    } else if (form.id === "approval-form") {
      const a = applications.find(a => a.id === state.application); a.approved = true;
      log(`Учебное одобрение ${a.name} ${a.version}. Реального artifact нет.`);
      modal.close(); renderContent();
    } else if (form.id === "new-app-form") {
      const name = String(data.get("name")).trim(); if (!name) return;
      const id = Math.max(...applications.map(a => a.id)) + 1;
      applications.push({id,name,initials:name.slice(0,2),description:"Пользовательский учебный пример",version:"1.0.0",previous:"—",kind:"json",path:"/version",approved:false});
      state.application = id; log("Добавлено учебное приложение."); modal.close(); renderContent();
    }
  });
  document.addEventListener("keydown", e => {
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") { e.preventDefault(); modal.open ? modal.close() : searchDialog(); }
    if (e.key === "F1") { e.preventDefault(); location.href = "../docs/index.html"; }
    if (modal.open && document.getElementById("global-search") && ["ArrowDown","ArrowUp"].includes(e.key)) {
      const buttons = [...modal.querySelectorAll("#global-results button")];
      if (!buttons.length) return; e.preventDefault();
      const idx = buttons.indexOf(document.activeElement);
      buttons[(idx + (e.key === "ArrowDown" ? 1 : -1) + buttons.length) % buttons.length].focus({preventScroll:true});
    }
  });
  modal.addEventListener("click", e => { if (e.target === modal) { const r = modal.getBoundingClientRect(); if (e.clientX < r.left || e.clientX > r.right || e.clientY < r.top || e.clientY > r.bottom) modal.close(); } });
  modal.addEventListener("close", () => { if (modal.open) return; modal.querySelectorAll('input[type="password"]').forEach(x => { x.value = ""; }); modal.innerHTML = ""; });
  window.addEventListener("hashchange", () => { const page = location.hash.slice(1); if (pages[page] && page !== state.page) route(page); });
  if (pages[location.hash.slice(1)]) state.page = location.hash.slice(1);
  render();
})();
