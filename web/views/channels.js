import { api } from "../api.js";

export async function renderChannels(state) {
  const el = document.getElementById("channels-panel");
  if (!el) return;
  try {
    const ch = await api.channels();
    if (!ch.length) {
      el.innerHTML = `<p class="muted">No channels detected.</p>`;
      return;
    }
    el.innerHTML = `
      <h3 style="margin:0 0 14px;font-size:14px">Channels (${ch.length})</h3>
      <table>
        <thead><tr>
          <th>ID</th><th>Label</th><th>Frames</th><th>Total bytes</th>
          <th>First offset</th><th>Last offset</th><th>Codec</th>
        </tr></thead>
        <tbody>
          ${ch.map((c) => `<tr>
            <td class="num">${c.id}</td>
            <td>${c.label}</td>
            <td class="num">${c.frame_count}</td>
            <td class="num">${c.total_bytes.toLocaleString()}</td>
            <td class="num">0x${c.first_offset.toString(16)}</td>
            <td class="num">0x${c.last_offset.toString(16)}</td>
            <td>${c.codec}</td>
          </tr>`).join("")}
        </tbody>
      </table>
    `;
  } catch (e) {
    el.innerHTML = `<p class="muted">No case loaded.</p>`;
  }
}
