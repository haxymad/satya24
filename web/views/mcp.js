// MCP tab: AI provider + API key settings, the MCP client config, and the tool list.
// The key is stored by the backend in ~/.config/satya/settings.json (mode 0600);
// the browser only ever sees a masked hint.

import { api } from "../api.js";

const esc = (v) =>
  String(v ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));

async function call(method, url, body) {
  const r = await fetch(url, {
    method,
    headers: body ? { "Content-Type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const t = await r.text();
  if (!r.ok) throw new Error(t || r.statusText);
  return JSON.parse(t);
}

export async function renderMCP() {
  const el = document.getElementById("mcp-panel");
  if (!el) return;
  let s = { provider: "anthropic", model: "", key_set: false, key_hint: null, stored_at: "", mcp_command: "satya-mcp" };
  try { s = await call("GET", "/api/settings"); } catch { /* backend without settings */ }

  el.innerHTML = `
    <h3>AI provider</h3>
    <div class="mcp-card">
      <div class="mcp-row"><label>Provider</label>
        <select id="mcp-provider">
          <option value="anthropic">Anthropic (Claude)</option>
          <option value="openai">OpenAI</option>
        </select></div>
      <div class="mcp-row"><label>Model</label><input id="mcp-model" placeholder="optional, e.g. a model id from Test key" value="${esc(s.model)}"></div>
      <div class="mcp-row"><label>API key</label>
        <input id="mcp-key" type="password" autocomplete="off" spellcheck="false"
          placeholder="${s.key_set ? `saved (${esc(s.key_hint)}) - type to replace` : "paste your API key"}"></div>
      <div class="mcp-btns">
        <button class="primary" id="mcp-save">Save</button>
        <button id="mcp-test" ${s.key_set ? "" : "disabled"}>Test key</button>
        <button id="mcp-clear" ${s.key_set ? "" : "disabled"}>Remove key</button>
        <span class="mcp-msg" id="mcp-msg"></span>
      </div>
      <p class="muted" style="margin:10px 0 0">Stored on this computer only, at <code>${esc(s.stored_at)}</code>
        (readable by your user account only). It is never written into case files or reports.</p>
    </div>

    <h3>Connect an AI client to SATYA</h3>
    <div class="mcp-card">
      <p class="muted" style="margin-top:0">Add this to your MCP client configuration (e.g. Claude Desktop) so it can call SATYA's forensic tools:</p>
      <pre class="mcp-config" id="mcp-config"></pre>
    </div>

    <h3>Available tools</h3>
    <div class="mcp-card" id="mcp-tools"><p class="muted">loading…</p></div>`;

  el.querySelector("#mcp-provider").value = s.provider || "anthropic";
  el.querySelector("#mcp-config").textContent = JSON.stringify(
    { mcpServers: { satya: { command: s.mcp_command, args: [] } } }, null, 2);

  const msg = (t, ok) => {
    const m = el.querySelector("#mcp-msg");
    m.textContent = t;
    m.className = "mcp-msg " + (ok === true ? "ok" : ok === false ? "err" : "");
  };

  el.querySelector("#mcp-save").onclick = async () => {
    const key = el.querySelector("#mcp-key").value.trim();
    const body = { provider: el.querySelector("#mcp-provider").value, model: el.querySelector("#mcp-model").value.trim() };
    if (key) body.api_key = key;
    try {
      const r = await call("POST", "/api/settings", body);
      msg(r.key_set ? `Saved (${r.key_hint})` : "Saved (no key)", true);
      renderMCP();
    } catch (e) { msg(e.message, false); }
  };
  el.querySelector("#mcp-clear").onclick = async () => {
    if (!confirm("Remove the saved API key?")) return;
    try { await call("POST", "/api/settings", { api_key: "" }); renderMCP(); } catch (e) { msg(e.message, false); }
  };
  el.querySelector("#mcp-test").onclick = async () => {
    msg("testing…");
    try {
      const r = await call("POST", "/api/settings/test");
      if (r.ok) {
        msg(`Key works - ${r.models.length} models available`, true);
        if (r.models.length && !el.querySelector("#mcp-model").value) el.querySelector("#mcp-model").placeholder = `e.g. ${r.models[0]}`;
      } else msg(r.error, false);
    } catch (e) { msg(e.message, false); }
  };

  try {
    const tools = await api.mcpTools();
    el.querySelector("#mcp-tools").innerHTML = tools.map((t) => `<div class="mcp-tool">
        <div class="tname">${esc(t.name)}</div><div class="tdesc">${esc(t.description || "")}</div></div>`).join("");
  } catch {
    el.querySelector("#mcp-tools").innerHTML = `<p class="muted">Tool list unavailable.</p>`;
  }
}
