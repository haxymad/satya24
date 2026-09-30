export function renderTree(state) {
  const tree = document.getElementById("tree");
  const f = state.frames;
  const counts = {
    all: f.length,
    allocated: f.filter((x) => x.recovery_source === "Allocated").length,
    carved: f.filter((x) => x.recovery_source === "Carved").length,
    partial: f.filter((x) => x.recovery_source === "Partial").length,
  };
  const byClaimSource = {};
  for (const fr of f) for (const c of fr.claims || []) {
    byClaimSource[c.source] = (byClaimSource[c.source] || 0) + 1;
  }
  const item = (key, label, count, icon = "•") => `
    <div class="tree-item" data-filter="${key}">
      <span><span class="ico">${icon}</span>${label}</span>
      <span class="count">${count}</span>
    </div>`;
  tree.innerHTML = `
    <div class="tree-group">Frames</div>
    ${item("all", "All frames", counts.all, "▣")}
    ${item("allocated", "Allocated", counts.allocated, "▤")}
    ${item("carved", "Carved (deleted)", counts.carved, "▨")}
    ${item("partial", "Partial", counts.partial, "▧")}
    <div class="tree-group">Timestamp sources</div>
    ${Object.entries(byClaimSource).map(([k, v]) => item("claim:" + k, k, v, "◔")).join("")}
    <div class="tree-group">Case</div>
    ${item("timeline", "Trust Engine", state.timeline.length, "◈")}
    ${item("custody", "Custody entries", 0, "⚿")}
  `;
  tree.querySelectorAll(".tree-item").forEach((el) => {
    el.addEventListener("click", () => {
      tree.querySelectorAll(".tree-item").forEach((x) => x.classList.remove("active"));
      el.classList.add("active");
      const key = el.dataset.filter;
      if (key.startsWith("claim:")) {
        state.filterSource = "all";
        state.search = key.slice(6).toLowerCase();
      } else if (key === "timeline") {
        document.querySelector('[data-view="timeline"]').click(); return;
      } else if (key === "custody") {
        document.querySelector('[data-view="custody"]').click(); return;
      } else { state.filterSource = key; }
      document.dispatchEvent(new CustomEvent("frame-filter-changed"));
    });
  });
}
