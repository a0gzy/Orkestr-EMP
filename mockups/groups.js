/* Offline groups prototype for layout A. All data and changes are in memory. */
"use strict";
window.OrkestrGroups = {
  create({devices, applications, esc, icon, tag, notice, refresh, navigate, showDevice, showApplication, openDialog, closeDialog, log}) {
    const groups = [
      {id:"apps",name:"Приложения",type:"folder",parentId:null,rules:[],members:[]},
      {id:"chrome",name:"Chrome",type:"folder",parentId:"apps",rules:[],members:[]},
      {id:"chrome-current", name:"Последняя версия", type:"smart", parentId:"chrome", match:"all", rules:[{field:"software", appId:5, op:"equal", reference:"latest"}], members:[]},
      {id:"chrome-old", name:"Старая версия", type:"smart", parentId:"chrome", match:"all", rules:[{field:"software", appId:5, op:"older", reference:"latest"}], members:[]},
      {id:"chrome-unknown", name:"Нет данных", type:"smart", parentId:"chrome", match:"all", rules:[{field:"software", appId:5, op:"unknown", reference:"latest"}], members:[]},
      {id:"hardware",name:"Оборудование",type:"folder",parentId:null,rules:[],members:[]},
      {id:"memory",name:"Оперативная память",type:"folder",parentId:"hardware",rules:[],members:[]},
      {id:"ram16", name:"16 ГБ и более", type:"smart", parentId:"memory", match:"all", rules:[{field:"ram", op:"ge", value:16}], members:[]},
      {id:"ram8", name:"8 ГБ и более", type:"smart", parentId:"memory", match:"all", rules:[{field:"ram", op:"ge", value:8}], members:[]},
      {id:"ram8to16", name:"От 8 до менее 16 ГБ", type:"smart", parentId:"memory", match:"all", rules:[{field:"ram", op:"ge", value:8},{field:"ram", op:"lt", value:16}], members:[]},
      {id:"ramlow", name:"Менее 8 ГБ", type:"smart", parentId:"memory", match:"all", rules:[{field:"ram", op:"lt", value:8}], members:[]},
      {id:"manual",name:"Ручные группы",type:"folder",parentId:null,rules:[],members:[]},
      {id:"pilot", name:"Пилотная группа", type:"manual", parentId:"manual", match:"all", rules:[], members:[1,5]}
    ];
    let selected = "chrome-old";
    let draft = null;
    let search = "";
    let nextId = 1;
    let deleted = null;
    const expanded = new Set(["apps","chrome","manual"]);
    const initialVersions = new Map(applications.map(a => [a.id,a.version]));
    const numericOps = {ge:"не менее (≥)",gt:"больше (>)",eq:"равно (=)",le:"не более (≤)",lt:"меньше (<)"};
    const softwareOps = {equal:"равна эталону",older:"ниже эталона",newer:"выше эталона",installed:"установлено",absent:"не установлено",unknown:"нет достоверных данных"};
    const clone = value => JSON.parse(JSON.stringify(value));
    const current = () => groups.find(g => g.id === selected) || groups[0];
    const children = id => id ? groups.filter(g => g.parentId === id) : [];
    function descendants(id,seen=new Set()) {
      if (!id || seen.has(id)) return [];
      seen.add(id);
      return children(id).flatMap(g => [g,...descendants(g.id,seen)]);
    }
    function path(group) {
      const result=[], seen=new Set();
      while (group && !seen.has(group.id)) { result.unshift(group); seen.add(group.id); group=groups.find(g => g.id === group.parentId); }
      return result;
    }
    const appFor = group => applications.find(a => a.id === [group,...descendants(group.id)].flatMap(g => g.rules).find(r => r.field === "software")?.appId) || applications.find(a => a.id === 5);
    const options = (entries, value) => entries.map(([v,t]) => `<option value="${esc(v)}" ${String(v) === String(value) ? "selected" : ""}>${esc(t)}</option>`).join("");
    const defaultRule = field => field === "software" ? {field,appId:5,op:"older",reference:"latest"} : field === "ram" ? {field,op:"ge",value:16} : {field,op:field === "source" ? "eq" : "contains",value:field === "source" ? "AD" : "Windows"};

    function compareVersions(a, b) {
      if (![a,b].every(v => typeof v === "string" && v.length < 128 && /^\d+(?:\.\d+){1,3}$/.test(v))) return null;
      const left = a.split(".").map(BigInt), right = b.split(".").map(BigInt);
      for (let i=0; i<Math.max(left.length,right.length); i++) {
        const x=left[i] ?? 0n, y=right[i] ?? 0n;
        if (x !== y) return x > y ? 1 : -1;
      }
      return 0;
    }
    function softwareStatus(device, application, reference = "latest") {
      if (!application || !device.installed) return {kind:"unknown",version:"—",reference:"—",label:"Нет данных",tone:"warn"};
      const version = device.installed[application.id];
      const target = reference === "approved" ? (application.approved ? application.version : null) : application.version;
      if (version === undefined) return {kind:"absent",version:"Не установлено",reference:target || "—",label:"Не установлено",tone:""};
      const compared = compareVersions(version,target);
      if (compared === null) return {kind:"unknown",version,reference:target || "Нет одобрения",label:"Неизвестно",tone:"warn"};
      const kind = compared === 0 ? "equal" : compared < 0 ? "older" : "newer";
      return {kind,version,reference:target,label:({equal:"Актуально",older:"Ниже эталона",newer:"Выше эталона"})[kind],tone:kind === "equal" ? "good" : kind === "older" ? "warn" : "signal"};
    }
    function evaluateRule(device, rule) {
      if (rule.field === "software") {
        const app = applications.find(a => a.id === rule.appId);
        const status = softwareStatus(device,app,rule.reference);
        if (rule.op === "unknown") return status.kind === "unknown";
        if (status.kind === "unknown") return null;
        if (rule.op === "installed") return status.kind !== "absent";
        return status.kind === rule.op;
      }
      if (rule.field === "ram") {
        if (typeof device.ramGB !== "number" || typeof rule.value !== "number" || !Number.isFinite(rule.value)) return null;
        const x=device.ramGB, y=rule.value;
        return ({ge:x>=y,gt:x>y,eq:x===y,le:x<=y,lt:x<y})[rule.op] ?? null;
      }
      const raw = rule.field === "source" ? device.source : rule.field === "os" ? device.os : device.name;
      if (!raw || raw === "—") return null;
      const text=raw.toLowerCase(), value=String(rule.value).toLowerCase();
      return rule.op === "eq" ? text === value : text.includes(value);
    }
    function evaluate(device, group, seen=new Set()) {
      if (seen.has(group.id)) return null;
      seen.add(group.id);
      if (group.type === "folder") {
        const values=children(group.id).map(g => evaluate(device,g,new Set(seen)));
        return values.includes(true) ? true : values.includes(null) ? null : false;
      }
      if (group.type === "manual") return group.members.includes(device.id);
      if (!group.rules.length) return null;
      const values = group.rules.map(rule => evaluateRule(device,rule));
      if (group.match === "any") return values.includes(true) ? true : values.includes(null) ? null : false;
      return values.includes(false) ? false : values.includes(null) ? null : true;
    }
    function membership(group) {
      const matched=[], unknown=[];
      for (const device of devices) { const value=evaluate(device,group); if (value === true) matched.push(device); else if (value === null) unknown.push(device); }
      return {matched,unknown};
    }
    function ruleLabel(rule) {
      if (rule.field === "ram") return `RAM ${({ge:"≥",gt:">",eq:"=",le:"≤",lt:"<"})[rule.op]} ${rule.value} ГБ`;
      if (rule.field === "software") {
        const name=applications.find(a => a.id === rule.appId)?.name || "Приложение";
        const reference=rule.reference === "approved" ? "одобренной версии" : "версии библиотеки";
        return `${name}: ${softwareOps[rule.op]}${["equal","older","newer"].includes(rule.op) ? ` ${reference}` : ""}`;
      }
      return `${({source:"Источник",os:"ОС",name:"Имя ПК"})[rule.field]} ${rule.op === "eq" ? "=" : "содержит"} ${rule.value}`;
    }
    function sidebarNode(group) {
      const name=esc(group.name), count=membership(group).matched.length;
      const button=`<button class="group-shortcut" data-group-id="${esc(group.id)}" title="${esc(path(group).map(g => g.name).join(" / "))}" ${group.id === selected ? 'aria-current="true"' : ""}><span class="group-dot" aria-hidden="true"></span><span class="group-nav-label">${name}</span><span class="group-count">${count}</span></button>`;
      if (group.type !== "folder") return button;
      return `<div class="group-folder-row"><button class="group-folder-toggle ${expanded.has(group.id) ? "is-open" : ""}" data-action="group-toggle" data-folder-id="${esc(group.id)}" aria-expanded="${expanded.has(group.id)}" aria-label="${expanded.has(group.id) ? "Свернуть" : "Раскрыть"} ${name}">${icon("arrow")}</button>${button}</div><div class="group-children" ${expanded.has(group.id) ? "" : "hidden"}>${children(group.id).map(sidebarNode).join("")}</div>`;
    }
    function sidebar() {
      return `<div class="sidebar-groups" aria-label="Группы компьютеров"><p class="group-category">Дерево групп</p>${groups.filter(g => !g.parentId).map(sidebarNode).join("")}<button class="group-shortcut" data-action="group-new">${icon("plus")}Создать группу</button></div>`;
    }
    function table(rows, group) {
      const application=appFor(group), reference=group.rules.find(r => r.field === "software")?.reference || "latest";
      if (!rows.length) return '<div class="empty"><strong>Нет подходящих компьютеров</strong>Измените правила или строку поиска. Неизвестные данные не считаются нулём.</div>';
      return `<table class="groups-table"><thead><tr><th>Компьютер</th><th>RAM</th><th>Установлено</th><th>Эталон</th><th>Состояние ПО</th></tr></thead><tbody>${rows.map(device => {
        const s=softwareStatus(device,application,reference);
        return `<tr><td><button class="row-link" data-group-device="${device.id}">${esc(device.name)}</button><span class="muted">${device.source === "AD" ? "Домен" : "Вне домена"}</span></td><td data-label="RAM">${esc(device.ram)}</td><td data-label="Установлено"><span class="mono">${esc(s.version)}</span></td><td data-label="Эталон"><span class="mono">${esc(s.reference)}</span></td><td data-label="Состояние ПО">${tag(s.label,s.tone,s.kind === "equal" ? "check" : "info")}</td></tr>`;
      }).join("")}</tbody></table>`;
    }
    function filtered(group) {
      return membership(group).matched.filter(d => `${d.name} ${d.host} ${d.ip} ${d.ram}`.toLowerCase().includes(search.toLowerCase()));
    }
    function view() {
      const group=current(), application=appFor(group), m=membership(group);
      return `<div class="group-heading"><div><p class="small muted group-path">${esc(path(group).map(g => g.name).join(" / "))}</p><div class="group-title-line">${tag(({folder:"Папка с подгруппами",smart:"Умная группа",manual:"Ручная группа"})[group.type],group.type === "smart" ? "signal" : "")}<span class="small muted">${({folder:"Объединение вложенных групп без дубликатов",smart:"Состав обновляется по условиям",manual:"Состав выбираете вы"})[group.type]}</span></div><h2>${esc(group.name)}</h2></div><div class="actions"><button data-action="group-edit">${icon("sliders")}Изменить</button><button class="primary" data-action="group-new">${icon("plus")}${group.type === "folder" ? "Создать подгруппу" : "Создать группу"}</button></div></div>
        <div class="group-switcher"><div class="group-switch-controls"><label>Открыть группу<select id="group-select">${options(groups.map(g => [g.id,`${path(g).map(p => p.name).join(" / ")} (${membership(g).matched.length})`]),selected)}</select></label><button data-action="group-tree">${icon("groups")}Дерево групп</button></div></div>
        ${deleted ? `<div class="group-undo"><span>Группа удалена. Компьютеры сохранены.</span><button data-action="group-undo">Отменить</button></div>` : ""}
        <div class="summary group-summary"><div><strong id="group-matched-count">${m.matched.length}</strong><span>ПК в группе</span></div><div><strong>${m.unknown.length}</strong><span>не определено по условиям</span></div><div><strong>${devices.length}</strong><span>ПК в учебной сети</span></div></div>
        <div class="group-expression">${group.type === "smart" ? `<span class="small muted">${group.match === "all" ? "Все условия (И)" : "Любое условие (ИЛИ)"}</span>${group.rules.map(r => `<span class="rule-pill">${esc(ruleLabel(r))}</span>`).join("")}` : group.type === "folder" ? children(group.id).map(g => `<button class="quiet" data-group-id="${esc(g.id)}">${esc(g.name)} ${tag(membership(g).matched.length)}</button>`).join("") || '<span class="muted">Подгрупп пока нет. Создайте первую.</span>' : `<span class="small muted">Участники выбраны вручную. Изменение ПО или RAM не меняет состав.</span>`}</div>
        <section class="panel"><div class="panel-head"><div><h2>Состав группы</h2><p class="small muted">ПО в таблице: ${esc(application.name)} · stable / x64 · учебные данные</p></div><label class="small">Найти компьютер<input id="group-search" type="search" value="${esc(search)}" placeholder="Имя, IP или RAM"></label></div><div id="group-results">${table(filtered(group),group)}</div><div class="table-bottom"><span id="group-result-count">Показано ${filtered(group).length} из ${m.matched.length}</span><button class="quiet" data-action="group-library">Эталон в библиотеке ${icon("arrow")}</button></div></section>
        ${m.unknown.length ? `<details class="group-unknown"><summary>Не удалось проверить условия: ${m.unknown.length} ПК</summary><p class="small muted">Нет достоверного скана, значения или эталона. Это не совпадения и не «ПО отсутствует».</p><div class="group-unknown-list">${m.unknown.map(d => `<button class="quiet" data-group-device="${d.id}">${esc(d.name)}</button>`).join("")}</div></details>` : ""}
        <details class="group-simulator"><summary>Проверить связь с библиотекой</summary><p class="small muted">Измените учебный эталон ${esc(application.name)}. Умные группы пересчитаются, ручные сохранят состав. Никаких запросов или установок.</p><div class="group-simulator-controls"><label>Последняя известная версия<input id="group-demo-version" value="${esc(application.version)}" maxlength="64" inputmode="decimal"></label><button data-action="group-version">Применить в демо</button><button class="quiet" data-action="group-version-reset">Сбросить версию</button></div><div id="group-version-result" class="result" role="status"></div></details>`;
    }
    function renderRule(rule,index) {
      const attrs = key => `data-rule-key="${key}"`;
      let controls="";
      if (rule.field === "ram") controls=`<label>Сравнение<select ${attrs("op")}>${options(Object.entries(numericOps),rule.op)}</select></label><label>Объём, ГБ<input ${attrs("value")} type="number" min="0" max="1048576" step="any" value="${esc(rule.value)}" required></label>`;
      else if (rule.field === "software") controls=`<label>Приложение<select ${attrs("appId")}>${options(applications.map(a => [a.id,a.name]),rule.appId)}</select></label><label>Условие<select ${attrs("op")}>${options(Object.entries(softwareOps),rule.op)}</select></label>${["equal","older","newer"].includes(rule.op) ? `<label>Сравнить с<select ${attrs("reference")}>${options([["latest","Последней известной в библиотеке"],["approved","Последней одобренной"]],rule.reference)}</select></label>` : ""}`;
      else if (rule.field === "source") controls=`<label>Источник<select ${attrs("value")}>${options([["AD","Домен / AD"],["Workgroup","Вне домена"]],rule.value)}</select></label>`;
      else controls=`<label>Сравнение<select ${attrs("op")}>${options([["contains","Содержит"],["eq","Равно"]],rule.op)}</select></label><label>Значение<input ${attrs("value")} maxlength="100" value="${esc(rule.value)}" required></label>`;
      return `<div class="group-rule" data-rule="${index}"><div class="group-rule-top"><label>Поле<select ${attrs("field")}>${options([["software","Приложение / версия"],["ram","Оперативная память"],["source","Источник ПК"],["os","Операционная система"],["name","Имя компьютера"]],rule.field)}</select></label><button type="button" class="quiet" data-action="group-rule-remove" data-rule-index="${index}" aria-label="Удалить условие ${index+1}" ${draft.rules.length === 1 ? "disabled" : ""}>Убрать</button></div><div class="group-rule-fields">${controls}</div></div>`;
    }
    function editorBody() {
      const excluded=new Set([draft.id,...descendants(draft.id).map(g => g.id)]);
      const parents=groups.filter(g => g.type === "folder" && !excluded.has(g.id));
      const hasChildren=draft.id && children(draft.id).length;
      return `<form id="group-form" class="modal-body"><div class="fields-two"><label>Название группы<input name="groupName" maxlength="80" value="${esc(draft.name)}" placeholder="Например: Последняя версия" required></label><label>Как формировать состав<select id="group-type">${options(hasChildren ? [["folder","Папка — вложенные группы"]] : [["smart","Умная — по условиям"],["manual","Ручная — выбрать ПК"],["folder","Папка — вложенные группы"]],draft.type)}</select></label></div><label>Расположение / родительская группа<select id="group-parent">${options([["","В корне дерева"],...parents.map(g => [g.id,path(g).map(p => p.name).join(" / ")])],draft.parentId || "")}</select><small>Можно создавать папки внутри папок и переносить группы. Правила не наследуются неявно.</small></label><div class="group-editor-grid"><section><h3>${({smart:"Правила включения",manual:"Участники группы",folder:"Вложенные группы"})[draft.type]}</h3><div id="group-builder">${builder()}</div></section><section class="group-preview" aria-label="Предпросмотр состава"><div id="group-preview" aria-live="polite">${preview()}</div></section></div><div id="group-save-result" class="result" role="status">Изменения в памяти макета. В приложении группы будут храниться на сервере.</div></form>`;
    }
    function builder() {
      if (draft.type === "folder") return `<p class="small muted">Папка объединяет компьютеры из вложенных групп без повторов. Условия каждой умной группы независимы.</p><div class="group-preview-list">${children(draft.id).map(g => `<div class="group-preview-row"><strong>${esc(g.name)}</strong><span>${membership(g).matched.length} ПК · ${({folder:"Папка",smart:"Умная",manual:"Ручная"})[g.type]}</span></div>`).join("") || '<p class="helper">Сохраните папку, затем выберите «Создать подгруппу».</p>'}</div>`;
      if (draft.type === "manual") return `<p class="small muted">Отметьте участников. Состав не зависит от фильтров и версий.</p><div class="group-member-list">${devices.map(d => `<label class="check"><input type="checkbox" data-member-id="${d.id}" ${draft.members.includes(d.id) ? "checked" : ""}><span>${esc(d.name)}<small class="muted"> · ${esc(d.ram)}</small></span></label>`).join("")}</div>`;
      return `<label class="group-match">Компьютер должен выполнить<select id="group-match">${options([["all","Все условия (И)"],["any","Хотя бы одно условие (ИЛИ)"]],draft.match)}</select></label><div class="group-rule-list">${draft.rules.map(renderRule).join("")}</div><button type="button" class="quiet" data-action="group-rule-add" ${draft.rules.length >= 10 ? "disabled" : ""}>${icon("plus")}Добавить условие</button><p class="helper">Неизвестное значение не равно нулю. Версия привязана к библиотеке, а не зашита в правило.</p>`;
    }
    function preview() {
      const m=membership(draft), application=appFor(draft);
      return `<h3>Подходят <strong id="group-preview-count">${m.matched.length}</strong> из ${devices.length} ПК</h3><p class="small muted">${({smart:"Пересчитывается при изменении правил",manual:"Выбраны вручную",folder:"Состав вложенных групп без повторов"})[draft.type]} · Не определено: ${m.unknown.length}</p><div class="group-preview-list">${m.matched.length ? m.matched.map(d => {
        const s=softwareStatus(d,application);
        return `<div class="group-preview-row"><strong>${esc(d.name)}</strong><span>${esc(d.ram)} · ${esc(application.name)}: ${esc(s.version)}</span></div>`;
      }).join("") : '<p class="muted">Пока нет совпадений. Попробуйте изменить условия или выбрать ПК.</p>'}</div><p class="helper">Учебный снимок. Никакое сканирование или развёртывание не запускается.</p>`;
    }
    function readDraft() {
      const form=document.getElementById("group-form");
      if (!form || !draft) return;
      draft.name=form.querySelector('[name="groupName"]').value;
      draft.type=form.querySelector("#group-type").value;
      draft.parentId=form.querySelector("#group-parent").value || null;
      if (form.querySelector("#group-match")) draft.match=form.querySelector("#group-match").value;
      form.querySelectorAll("[data-rule]").forEach(el => {
        const rule=draft.rules[Number(el.dataset.rule)];
        if (!rule) return;
        el.querySelectorAll("[data-rule-key]").forEach(control => {
          const key=control.dataset.ruleKey;
          rule[key]=key === "appId" ? Number(control.value) : key === "value" && rule.field === "ram" ? (control.value.trim() === "" ? null : Number(control.value)) : control.value;
        });
      });
      const checks=[...form.querySelectorAll("[data-member-id]")];
      if (checks.length) draft.members=checks.filter(c => c.checked).map(c => Number(c.dataset.memberId));
    }
    function updatePreview() { const target=document.getElementById("group-preview"); if (target) target.innerHTML=preview(); }
    function updateBuilder() { document.getElementById("group-builder").innerHTML=builder(); updatePreview(); }
    function edit(isNew = false) {
      const group=current();
      draft=isNew ? {id:null,name:"",type:"smart",parentId:group.type === "folder" ? group.id : group.parentId,match:"all",rules:[defaultRule("software")],members:[]} : clone(group);
      showEditor(isNew);
    }
    function showEditor(isNew = !draft.id) {
      openDialog(isNew ? "Создать группу компьютеров" : "Изменить группу", "Папки и подгруппы, умные и ручные выборки. Предпросмотр на учебных данных.", editorBody(), `<button type="button" data-action="close">Отмена</button>${draft.id ? `<button class="quiet" type="button" data-action="group-delete" ${children(draft.id).length ? 'disabled title="Сначала перенесите вложенные группы"' : ""}>Удалить группу</button>` : ""}<button class="primary" type="submit" form="group-form">Сохранить группу</button>`);
      document.getElementById("modal").classList.add("group-editor-dialog");
    }
    function select(id) { if (groups.some(g => g.id === id)) { selected=id; search=""; path(current()).forEach(g => { if (g.type === "folder") expanded.add(g.id); }); } }
    function handleClick(button) {
      const d=button.dataset;
      if (d.groupId) { select(d.groupId); if (button.closest("#modal")) closeDialog(); navigate("groups"); return true; }
      if (d.groupDevice) { showDevice(Number(d.groupDevice)); return true; }
      if (!d.action?.startsWith("group-")) return false;
      switch(d.action) {
        case "group-toggle": {
          expanded.has(d.folderId) ? expanded.delete(d.folderId) : expanded.add(d.folderId);
          const navigator=document.getElementById("group-tree-navigator");
          if (navigator) navigator.innerHTML=sidebar(); else refresh(); break;
        }
        case "group-tree": openDialog("Дерево групп", "Папки и вложенные выборки. Выберите группу, чтобы открыть её состав.", `<div class="modal-body group-tree-navigator" id="group-tree-navigator">${sidebar()}</div>`); break;
        case "group-new": edit(true); break;
        case "group-edit": edit(); break;
        case "group-library": showApplication(appFor(current()).id); break;
        case "group-rule-add": readDraft(); if (draft.rules.length < 10) draft.rules.push(defaultRule("ram")); updateBuilder(); break;
        case "group-rule-remove": readDraft(); if (draft.rules.length > 1) draft.rules.splice(Number(d.ruleIndex),1); updateBuilder(); break;
        case "group-delete": {
          const index=groups.findIndex(g => g.id === draft.id);
          if (index < 0 || groups.length === 1 || children(draft.id).length) break;
          deleted={group:groups[index],index}; groups.splice(index,1); selected=groups[0].id;
          closeDialog(); log("Учебная группа удалена; компьютеры сохранены."); refresh(); break;
        }
        case "group-undo": if (deleted) { groups.splice(deleted.index,0,deleted.group); selected=deleted.group.id; deleted=null; refresh(); } break;
        case "group-version": {
          const input=document.getElementById("group-demo-version"), application=appFor(current());
          if (compareVersions(input.value.trim(),input.value.trim()) === null) { input.setAttribute("aria-invalid","true"); const r=document.getElementById("group-version-result"); r.dataset.state="error"; r.textContent="Нужна числовая версия: например 131.0.0.0."; break; }
          application.version=input.value.trim(); application.approved=false;
          log(`Изменён учебный эталон ${application.name}. Умные группы пересчитаны, установки нет.`);
          refresh(); break;
        }
        case "group-version-reset": { const application=appFor(current()); application.version=initialVersions.get(application.id); application.approved=false; refresh(); break; }
      }
      return true;
    }
    function handleChange(event) {
      const el=event.target;
      if (el.id === "group-select") { select(el.value); refresh(); return true; }
      if (!el.closest("#group-form")) return false;
      readDraft();
      if (el.id === "group-type") { if (draft.type === "smart" && !draft.rules.length) draft.rules.push(defaultRule("software")); showEditor(); }
      else if (el.dataset.ruleKey === "field") { const index=Number(el.closest("[data-rule]").dataset.rule); draft.rules[index]=defaultRule(el.value); updateBuilder(); }
      else if (el.dataset.ruleKey === "op") updateBuilder();
      else updatePreview();
      return true;
    }
    function handleInput(event) {
      if (event.target.id === "group-search") {
        search=event.target.value; const group=current();
        document.getElementById("group-results").innerHTML=table(filtered(group),group);
        document.getElementById("group-result-count").textContent=`Показано ${filtered(group).length} из ${membership(group).matched.length}`;
        return true;
      }
      if (event.target.closest("#group-form")) { readDraft(); updatePreview(); return true; }
      return false;
    }
    function save(form) {
      readDraft();
      if (!draft.name.trim()) return;
      if (draft.type === "smart" && draft.rules.some(r => r.field === "ram" && (r.value === null || !Number.isFinite(r.value) || r.value < 0))) {
        document.getElementById("group-save-result").textContent="Укажите неотрицательный числовой объём RAM."; return;
      }
      draft.name=draft.name.trim();
      const saved=clone(draft);
      if (saved.parentId && (!groups.some(g => g.id === saved.parentId && g.type === "folder") || saved.parentId === saved.id || descendants(saved.id).some(g => g.id === saved.parentId))) {
        document.getElementById("group-save-result").textContent="Недопустимый родитель: нельзя поместить группу внутрь самой себя или её подгруппы."; return;
      }
      if (children(saved.id).length && saved.type !== "folder") { document.getElementById("group-save-result").textContent="Сначала перенесите вложенные группы."; return; }
      if (!saved.id) { saved.id=`custom-${nextId++}`; groups.push(saved); }
      else groups[groups.findIndex(g => g.id === saved.id)]=saved;
      selected=saved.id; search=""; draft=null; deleted=null;
      path(saved).forEach(g => { if (g.type === "folder") expanded.add(g.id); });
      closeDialog(); log("Группа сохранена в памяти макета. Сетевых действий не было."); navigate("groups");
    }
    return {sidebar,view,select,handleClick,handleChange,handleInput,save,searchItems:() => groups.map(g => ({id:g.id,name:path(g).map(p => p.name).join(" / ")})),compareVersions};
  }
};
