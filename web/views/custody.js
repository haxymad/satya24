import { api } from "../api.js";
export function renderCustody(state) {
  api.custody().then((entries) => {
    const tbody = document.getElementById("custody-tbody");
    if (!entries || !entries.length) {
      tbody.innerHTML = `<tr><td colspan="5" class="empty">No custody entries yet</td></tr>`;
      return;
    }
    tbody.innerHTML = entries.map((e, i) => `<tr>
        <td class="num">${i}</td>
        <td>${e.ts}</td>
        <td>${JSON.stringify(e.event)}</td>
        <td>${(e.hash || "").slice(0, 24)}…</td>
        <td>${e.sig ? '<span class="badge ok">signed</span>' : "—"}</td>
      </tr>`).join("");
  }).catch(() => {});
}
