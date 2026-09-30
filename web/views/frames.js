export function renderFrames(state) {
  const tbody = document.getElementById("frames-tbody");
  const q = state.search.toLowerCase();
  let rows = state.frames;
  if (state.filterSource !== "all") {
    const map = { allocated: "Allocated", carved: "Carved", partial: "Partial" };
    const want = map[state.filterSource];
    if (want) rows = rows.filter((f) => f.recovery_source === want);
  }
  if (q) {
    rows = rows.filter((f) =>
      String(f.offset).includes(q) || String(f.length).includes(q) ||
      f.recovery_source.toLowerCase().includes(q) ||
      (f.claims || []).some((c) => c.source.toLowerCase().includes(q) || c.claimed_utc.includes(q))
    );
  }
  if (!rows.length) {
    tbody.innerHTML = `<tr><td colspan="9" class="empty">no matching frames</td></tr>`;
    return;
  }
  tbody.innerHTML = rows.slice(0, 2000).map((f, i) => {
    const originalIdx = state.frames.indexOf(f);
    const tl = state.timeline[originalIdx];
    const source = f.recovery_source.toLowerCase();
    const claims = f.claims || [];
    const ts = tl
      ? new Date(tl.mean_s * 1000).toISOString().replace("T", " ").slice(0, 19)
      : (claims[0] ? claims[0].claimed_utc.replace("T", " ").slice(0, 19) : "—");
    const ci = tl
      ? `[${new Date(tl.ci_lo_95_s * 1000).toISOString().slice(11, 19)}, ` +
        `${new Date(tl.ci_hi_95_s * 1000).toISOString().slice(11, 19)}]`
      : "—";
    const cal = tl ? (tl.collapsed ? "anchored" : "self-rep") : "—";
    const calClass = tl ? (tl.collapsed ? "cal-anchored" : "cal-self-reported") : "";
    return `<tr data-idx="${originalIdx}">
        <td class="num">${originalIdx}</td>
        <td class="num">0x${f.offset.toString(16).padStart(8, "0")}</td>
        <td class="num">${f.length}</td>
        <td>${f.codec}</td>
        <td><span class="badge ${source}">${f.recovery_source}</span></td>
        <td class="num">${f.recovery_confidence.toFixed(2)}</td>
        <td>${ts}</td>
        <td class="ci">${ci}</td>
        <td class="${calClass}">${cal}</td>
      </tr>`;
  }).join("");
  tbody.querySelectorAll("tr").forEach((tr) => {
    tr.addEventListener("click", () => {
      tbody.querySelectorAll("tr").forEach((x) => x.classList.remove("selected"));
      tr.classList.add("selected");
      const idx = parseInt(tr.dataset.idx, 10);
      document.dispatchEvent(new CustomEvent("frame-selected", {
        detail: { idx, frame: state.frames[idx], timeline: state.timeline[idx] },
      }));
    });
  });
}
