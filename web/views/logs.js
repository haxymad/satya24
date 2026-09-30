import { api } from "../api.js";

export async function renderLogs(state) {
  const el = document.getElementById("logs-panel");
  if (!el) return;
  try {
    const logs = await api.logs();
    if (!logs.length) {
      el.innerHTML = `<p class="muted">No log records extracted from this image.</p>`;
      return;
    }
    const counts = {};
    for (const l of logs) counts[l.category] = (counts[l.category] || 0) + 1;

    el.innerHTML = `
      <h3 style="margin:0 0 14px;font-size:14px">Device Logs (${logs.length})</h3>
      <p class="muted" style="margin-bottom:12px">
        ${Object.entries(counts).map(([k, v]) => `<span style="background:var(--bg-3);padding:2px 8px;border-radius:3px;margin-right:6px">${k}: ${v}</span>`).join("")}
      </p>
      <table>
        <thead><tr>
          <th style="width:60px">#</th><th style="width:110px">Offset</th>
          <th style="width:200px">Timestamp (UTC)</th>
          <th style="width:100px">Category</th><th>Description</th>
        </tr></thead>
        <tbody>
          ${logs.slice(0, 500).map((l, i) => `<tr>
            <td class="num">${i}</td>
            <td class="num">0x${l.offset.toString(16)}</td>
            <td>${l.timestamp_utc || "—"}</td>
            <td>${l.category}</td>
            <td>${l.description} (${l.major_type}/${l.minor_type})</td>
          </tr>`).join("")}
        </tbody>
      </table>
      ${logs.length > 500 ? `<p class="muted" style="margin-top:10px">Showing first 500 of ${logs.length} records.</p>` : ""}
    `;
  } catch (e) {
    el.innerHTML = `<p class="muted">No case loaded.</p>`;
  }
}
