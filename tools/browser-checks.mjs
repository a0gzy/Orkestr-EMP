/* Run only in a same-origin localhost browser during prototype verification.
 * Does not access endpoints, credentials or external networks. */
export async function runChecks({paths=["/mockups/sidebar.html","/mockups/tabs.html"],widths=[320,375,414,768,1024,1280,1920],includeGroupScenarios=true,includeDocs=true}={}) {
  const results = [];
  const issues = [];
  const frame = document.createElement("iframe");
  frame.title = "Проверка адаптивности макета";
  Object.assign(frame.style, {position:"absolute", top:"0", left:"0", height:"900px", border:"0", zIndex:"100"});
  document.body.append(frame);
  function assert(ok, name, detail = "") { (ok ? results : issues).push({name, ...(detail ? {detail} : {})}); }
  function load(path, width) {
    frame.style.width = `${width}px`;
    return new Promise((resolve, reject) => {
      frame.onload = () => resolve(frame.contentWindow);
      frame.onerror = reject;
      frame.src = new URL(path, location.origin).href;
    });
  }
  const rendered = el => el.getClientRects().length && getComputedStyle(el).visibility !== "hidden";
  function layoutChecks(w, name) {
    const d = w.document;
    assert(d.documentElement.scrollWidth <= w.innerWidth + 1, `${name}: no root horizontal overflow`, `${d.documentElement.scrollWidth}/${w.innerWidth}`);
    const overflow = [...d.querySelectorAll("button,input:not([type=checkbox]),select,textarea,table,pre")].filter(el => {
      if (!rendered(el)) return false;
      const r = el.getBoundingClientRect();
      // Single-line form values intentionally scroll inside their own input.
      const contentOverflow = !["INPUT", "TEXTAREA", "SELECT"].includes(el.tagName) && el.scrollWidth > el.clientWidth + 4;
      return r.right > w.innerWidth + 2 || r.left < -2 || contentOverflow;
    }).map(el => `${el.tagName}: ${(el.textContent || el.getAttribute("name") || "").trim().slice(0,65)}`);
    assert(!overflow.length, `${name}: controls/content fit`, overflow.join("; "));
    const clippedContainers = [...d.querySelectorAll(".sidebar,.panel")].filter(el => rendered(el) && el.scrollWidth > el.clientWidth + 2).map(el => el.className);
    assert(!clippedContainers.length, `${name}: containers do not clip content`, clippedContainers.join(", "));
    const unlabelled = [...d.querySelectorAll("input,textarea,select")].filter(el => rendered(el) && !el.closest("label") && !el.labels?.length && !el.getAttribute("aria-label"));
    assert(!unlabelled.length, `${name}: controls labelled`);
  }
  const nextFrame = w => new Promise(resolve => w.requestAnimationFrame(() => resolve()));
  async function waitFor(w, selector, predicate) {
    const el = w.document.querySelector(selector);
    if (predicate(el)) return el;
    return new Promise((resolve, reject) => {
      const observer = new w.MutationObserver(() => { if (predicate(el)) { observer.disconnect(); clearTimeout(timer); resolve(el); } });
      const timer = setTimeout(() => { observer.disconnect(); reject(new Error(`Condition timeout: ${selector}`)); }, 4000);
      observer.observe(el, {childList:true, subtree:true, attributes:true, characterData:true});
    });
  }
  function change(w, el, value) { el.value = value; el.dispatchEvent(new w.Event("change", {bubbles:true})); }
  try {
    for (const path of paths) {
      for (const width of widths) {
        const w = await load(path, width); const d = w.document;
        for (const page of ["computers",...(path.includes("sidebar") ? ["groups"] : []),"library","jobs","policies","activity","settings"]) {
          d.querySelector(`[data-page="${page}"]`).click();
          layoutChecks(w, `${path} ${width} ${page}`);
        }
        if (width === 320 || width === 1280) {
          for (const action of ["connect","credentials","search"]) {
            d.querySelector(`[data-action="${action}"]`).click();
            layoutChecks(w, `${path} ${width} dialog:${action}`);
            assert(d.querySelector("dialog").open, `${path} ${width} ${action}: modal open`);
            d.querySelector("dialog").close(); await nextFrame(w);
          }
        }
        if (path.includes("sidebar")) {
          d.querySelector('[data-page="groups"]').click();
          d.querySelector('[data-action="group-edit"]').click();
          layoutChecks(w,`${path} ${width} smart group editor`);
          change(w,d.querySelector("#group-type"),"manual");
          layoutChecks(w,`${path} ${width} manual group editor`);
          d.querySelector("dialog").close(); await nextFrame(w);
          change(w,d.querySelector("#group-select"),"chrome");
          layoutChecks(w,`${path} ${width} folder view`);
          d.querySelector('[data-action="group-edit"]').click();
          layoutChecks(w,`${path} ${width} folder editor`);
          d.querySelector("dialog").close(); await nextFrame(w);
          if (width < 1024) {
            d.querySelector('[data-action="group-tree"]').click();
            layoutChecks(w,`${path} ${width} group tree navigator`);
            d.querySelector("dialog").close(); await nextFrame(w);
          }
        }
      }
      const w = await load(path,1280); const d = w.document;
      d.querySelector('[data-page="computers"]').click();
      const input = d.querySelector("#list-search"); input.value = "WG-"; input.dispatchEvent(new w.Event("input", {bubbles:true}));
      assert(d.querySelectorAll("tbody tr").length === 2, `${path}: computer search`);
      d.querySelector('[data-page="library"]').click();
      for (const kind of ["json","html","text"]) {
        change(w,d.querySelector("#source-kind"),kind);
        d.querySelector('[data-action="test-source"]').click();
        await waitFor(w,"#source-result", el => ["error","success"].includes(el.dataset.state));
        assert(d.querySelector("#source-result").dataset.state === "success", `${path}: ${kind} fixture extraction`, d.querySelector("#source-result").textContent);
      }
      change(w,d.querySelector("#download-kind"),"link");
      d.querySelector('[data-action="test-source"]').click();
      await waitFor(w,"#source-result", el => ["error","success"].includes(el.dataset.state));
      assert(d.querySelector("#source-result").dataset.state === "success" && d.querySelector("#source-result").textContent.includes("/files/app-2.4.1-x64.msi"), `${path}: href plus regex extraction`);
      change(w,d.querySelector("#download-kind"),"template");
      change(w,d.querySelector("#source-kind"),"html");
      d.querySelector('[name="extractor"]').value = ".missing";
      d.querySelector('[data-action="test-source"]').click();
      await waitFor(w,"#source-result", el => el.dataset.state === "error");
      assert(d.querySelector("#source-result").dataset.state === "error", `${path}: missing selector rejected`);
      d.querySelector('[data-action="approve"]').click();
      const approve = d.querySelector("#approval-form"); approve.querySelector('[type="checkbox"]').checked = true; approve.requestSubmit(); await nextFrame(w);
      assert(d.querySelector('[data-action="approve"]').disabled, `${path}: approval demo`);
      d.querySelector('[data-page="jobs"]').click();
      assert(!d.querySelector('[data-action="plan"]').disabled, `${path}: planning unlocked only after approval`);
      d.querySelector('[data-action="plan"]').click();
      assert(d.querySelector("#plan-result").textContent.includes("Никаких задач"), `${path}: plan remains demo`);
      d.querySelector('[data-page="policies"]').click();
      d.querySelector('[name="global"]').value = "1"; d.querySelector('[name="site"]').value = "2"; d.querySelector("#policy-form").requestSubmit();
      assert(d.querySelector("#policy-result").dataset.state === "error", `${path}: invalid quotas rejected`);
      d.querySelector('[name="site"]').value = "1"; d.querySelector("#policy-form").requestSubmit();
      assert(d.querySelector("#policy-result").dataset.state === "success", `${path}: valid quotas accepted`);
      d.querySelector('[data-action="credentials"]').click();
      const f = d.querySelector("#credential-form");
      f.querySelector('[name="label"]').value = "Учебный профиль";
      f.querySelector('[name="username"]').value = "LAB\\demo";
      f.querySelector('[name="password"]').value = "DEMO-ONLY-NOT-A-SECRET";
      f.requestSubmit();
      assert(!f.querySelector('[name="password"]').value, `${path}: demo password cleared`);
      assert(!JSON.stringify({...w.localStorage}).includes("DEMO-ONLY-NOT-A-SECRET"), `${path}: password not persisted`);
      d.querySelector("dialog").close(); await nextFrame(w);
      change(w,d.querySelector("#theme"),"light"); assert(d.documentElement.dataset.mode === "light", `${path}: light theme`);
      change(w,d.querySelector("#theme"),"dark"); assert(d.documentElement.dataset.mode === "dark", `${path}: dark theme`);
      change(w,d.querySelector("#theme"),"system");
      d.querySelector('[data-action="search"]').click();
      const search = d.querySelector("#global-search"); search.value = "TEST-WG-01"; search.dispatchEvent(new w.Event("input", {bubbles:true}));
      assert(d.querySelectorAll("#global-results button").length === 1, `${path}: command search`);
      d.querySelector("#global-results button").click(); await nextFrame(w);
      assert(d.querySelector("aside[aria-label] h2").textContent === "TEST-WG-01", `${path}: search navigates to device`);
    }
    if (includeGroupScenarios && paths.some(p => p.includes("sidebar"))) {
      const w=await load("/mockups/sidebar.html",1280), d=w.document;
      d.querySelector('[data-page="groups"]').click();
      const count=()=>Number(d.querySelector("#group-matched-count").textContent);
      const selectGroup=id=>change(w,d.querySelector("#group-select"),id);
      const previewCount=()=>Number(d.querySelector("#group-preview-count").textContent);
      for (const [id,expected] of [["chrome-old",2],["chrome-current",2],["chrome-unknown",2],["ram16",4],["ram8",5],["ram8to16",1],["ramlow",1],["pilot",2]]) {
        selectGroup(id); assert(count() === expected,`groups: ${id} membership`,`${count()}/${expected}`);
      }
      selectGroup("chrome");assert(count()===6,"groups: Chrome folder is union of children");
      selectGroup("apps");assert(count()===6,"groups: nested folder union without duplicates");
      selectGroup("hardware");assert(count()===6,"groups: overlapping RAM subgroups deduplicated");
      d.querySelector('[data-action="group-edit"]').click();
      assert(![...d.querySelector("#group-parent").options].some(o=>["hardware","memory"].includes(o.value)),"groups: self and descendant parents excluded");
      assert(d.querySelector('[data-action="group-delete"]').disabled,"groups: folder with children protected from deletion");
      d.querySelector("dialog").close();await nextFrame(w);
      selectGroup("chrome-old");
      assert(d.querySelector("#group-results").textContent.includes("129.0.0.0") && d.querySelector("#group-results").textContent.includes("130.0.0.0"),"groups: installed and reference versions visible");
      const q=d.querySelector("#group-search");q.value="PC-04";q.dispatchEvent(new w.Event("input",{bubbles:true}));
      assert(d.querySelectorAll("#group-results tbody tr").length === 1,"groups: member search");
      d.querySelector('[data-action="group-new"]').click();
      d.querySelector('[name="groupName"]').value="Моя пилотная группа";
      change(w,d.querySelector("#group-type"),"manual");
      for(const id of [2,7]) {const c=d.querySelector(`[data-member-id="${id}"]`);c.checked=true;c.dispatchEvent(new w.Event("change",{bubbles:true}));}
      assert(previewCount()===2,"groups: manual live preview");
      d.querySelector("#group-form").requestSubmit();await nextFrame(w);
      const manualId=d.querySelector("#group-select").value;
      assert(count()===2 && d.querySelector(".group-heading h2").textContent === "Моя пилотная группа","groups: save manual");
      d.querySelector('[data-action="group-new"]').click();
      d.querySelector('[name="groupName"]').value="RAM от 8 до 16";
      change(w,d.querySelector('[data-rule-key="field"]'),"ram");
      const v=d.querySelector('[data-rule-key="value"]');v.value="8";v.dispatchEvent(new w.Event("input",{bubbles:true}));
      assert(previewCount()===5,"groups: numeric >=8 preview");
      d.querySelector('[data-action="group-rule-add"]').click();
      change(w,d.querySelector('[data-rule="1"] [data-rule-key="op"]'),"lt");
      assert(previewCount()===1,"groups: AND range excludes >=16");
      change(w,d.querySelector("#group-match"),"any");
      assert(previewCount()===6,"groups: OR three-valued evaluation");
      change(w,d.querySelector("#group-match"),"all");
      d.querySelector("#group-form").requestSubmit();await nextFrame(w);
      const smartId=d.querySelector("#group-select").value;
      assert(count()===1,"groups: save smart rules");
      selectGroup("chrome-old");d.querySelector("#group-demo-version").value="131.0.0.0";d.querySelector('[data-action="group-version"]').click();
      assert(count()===4,"groups: library change recomputes old versions");
      selectGroup("chrome-current");assert(count()===0,"groups: latest group recomputed without copied version");
      selectGroup(manualId);assert(count()===2,"groups: manual membership unaffected by library");
      selectGroup(smartId);d.querySelector('[data-action="group-edit"]').click();d.querySelector('[data-action="group-delete"]').click();await nextFrame(w);
      assert(![...d.querySelector("#group-select").options].some(o=>o.value===smartId),"groups: delete group only");
      d.querySelector('[data-action="group-undo"]').click();assert(d.querySelector("#group-select").value===smartId,"groups: undo deletion");
      selectGroup("chrome-old");d.querySelector('[data-action="group-version-reset"]').click();assert(count()===2,"groups: reset reference");
      selectGroup(manualId);d.querySelector('[data-action="group-edit"]').click();
      change(w,d.querySelector("#group-parent"),"manual");d.querySelector("#group-form").requestSubmit();await nextFrame(w);
      assert(d.querySelector(".group-path").textContent.includes("Ручные группы"),"groups: move subgroup to another parent");
      selectGroup("chrome");d.querySelector('[data-action="group-new"]').click();
      assert(d.querySelector("#group-parent").value === "chrome","groups: create subgroup uses selected folder");
      d.querySelector('[name="groupName"]').value="Тестовая вложенная папка";change(w,d.querySelector("#group-type"),"folder");
      d.querySelector("#group-form").requestSubmit();await nextFrame(w);const nestedId=d.querySelector("#group-select").value;
      d.querySelector('[data-action="group-new"]').click();assert(d.querySelector("#group-parent").value===nestedId,"groups: deeper nesting supported");
      d.querySelector('[name="groupName"]').value="Версия ниже эталона";d.querySelector("#group-form").requestSubmit();await nextFrame(w);
      assert(count()===2 && d.querySelector(".group-path").textContent.includes("Тестовая вложенная папка"),"groups: nested smart subgroup saved");
      selectGroup("chrome-old");
      d.querySelector('[data-group-device="1"]').click();assert(d.querySelector("aside[aria-label] h2").textContent === "TEST-PC-01","groups: open member card");
      assert(d.querySelector("aside[aria-label]").textContent.includes("Google Chrome"),"groups: installed software in card");
    }
    if (includeDocs) for (const path of ["/index.html","/docs/index.html"]) for (const width of [320,375,414,768,1280]) {
      const w = await load(path,width); layoutChecks(w,`${path} ${width}`);
      if (path.includes("docs")) {
        const input = w.document.querySelector("#help-search"); input.value = "DPAPI"; input.dispatchEvent(new w.Event("input", {bubbles:true}));
        assert(w.document.querySelectorAll("article:not([hidden])").length > 0 && w.document.querySelectorAll("article[hidden]").length > 0, `${path} ${width}: offline full-text search`);
      }
    }
  } catch (e) { issues.push({name:"Test harness exception",detail:e.stack}); }
  frame.remove();
  return {passed:results.length,failed:issues.length,issues};
}
