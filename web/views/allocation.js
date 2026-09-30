import { api } from "../api.js";

export async function renderAllocation(state) {
  const el = document.getElementById("allocation-tbody");
  if (!el) return;
  try {
    const regions = await api.allocation();
    const total = regions.length ? regions[regions.length - 1].end : 1;

    // Canvas bar
    const canvas = document.getElementById("allocation-canvas");
    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    const ctx = canvas.getContext("2d");
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, rect.width, rect.height);

    for (const r of regions) {
      const x0 = (r.start / total) * rect.width;
      const x1 = (r.end / total) * rect.width;
      ctx.fillStyle =
        r.kind === "allocated" ? "#6fcf6f" :
        r.kind === "unallocated" ? "#333337" :
        r.kind === "log" ? "#4f9dff" : "#858585";
      ctx.fillRect(x0, 0, x1 - x0, rect.height);
    }

    el.innerHTML = regions.slice(0, 200).map((r) => `<tr>
        <td class="num">0x${r.start.toString(16)}</td>
        <td class="num">0x${r.end.toString(16)}</td>
        <td class="num">${((r.end - r.start) / 1e6).toFixed(1)} MB</td>
        <td>${r.kind}</td>
        <td class="num">${r.frame_count}</td>
      </tr>`).join("");

    if (regions.length > 200) {
      el.innerHTML += `<tr><td colspan="5" class="muted" style="text-align:center;padding:10px">… ${regions.length - 200} more regions</td></tr>`;
    }
  } catch (e) {
    el.innerHTML = `<tr><td colspan="5" class="muted">No case loaded.</td></tr>`;
  }
}
