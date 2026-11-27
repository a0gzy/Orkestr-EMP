"use strict";
(() => {
  const root = document.documentElement;
  const control = document.getElementById("help-theme");
  const media = matchMedia("(prefers-color-scheme: dark)");
  let theme = "system";
  try { theme = localStorage.getItem("orkestr-demo-theme") || "system"; } catch (_) {}
  if (!["system", "light", "dark"].includes(theme)) theme = "system";
  const apply = () => { root.dataset.mode = theme === "system" ? (media.matches ? "dark" : "light") : theme; };
  apply(); media.addEventListener("change", apply);
  if (control) { control.value = theme; control.addEventListener("change", () => { theme = control.value; try { localStorage.setItem("orkestr-demo-theme", theme); } catch (_) {} apply(); }); }
  const input = document.getElementById("help-search");
  if (!input) return;
  const articles = [...document.querySelectorAll("article[id]")];
  const links = [...document.querySelectorAll(".help-nav nav a")];
  function filter() {
    const query = input.value.toLowerCase().trim();
    let count = 0;
    for (const article of articles) { const matches = !query || article.textContent.toLowerCase().includes(query); article.hidden = !matches; if (matches) count++; }
    document.getElementById("help-result").textContent = query ? `Найдено разделов: ${count}. Очистите поиск, чтобы увидеть все.` : "Офлайн-поиск по тексту всех разделов.";
  }
  input.addEventListener("input", filter);
  function highlight() { for (const link of links) { if (link.hash === location.hash) link.setAttribute("aria-current", "location"); else link.removeAttribute("aria-current"); } }
  for (const link of links) link.addEventListener("click", () => { input.value = ""; filter(); });
  window.addEventListener("hashchange", () => { input.value = ""; filter(); highlight(); });
  document.querySelectorAll("td").forEach(td => { const table = td.closest("table"); const heads = table?.querySelectorAll("thead th"); if (heads?.[td.cellIndex]) td.dataset.label = heads[td.cellIndex].textContent; });
  highlight(); filter();
})();
