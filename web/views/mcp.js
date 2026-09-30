import { api } from "../api.js";
export function renderMCP(state) {
  document.getElementById("mcp-config").textContent = JSON.stringify({
    mcpServers: { satya: {
      command: "/home/haxy/satya/satya/target/release/satya-mcp", args: [],
    }},
  }, null, 2);
  api.mcpTools().then((tools) => {
    const el = document.getElementById("mcp-tools");
    if (!tools || !tools.length) {
      el.innerHTML = `<p class="muted" style="padding:16px">No MCP tools available.</p>`;
      return;
    }
    el.innerHTML = tools.map((t) => `<div class="mcp-tool">
        <div class="tname">${t.name}</div>
        <div class="tdesc">${t.description || ""}</div>
      </div>`).join("");
  }).catch(() => {
    document.getElementById("mcp-tools").innerHTML =
      `<p class="muted" style="padding:16px">MCP server not running.</p>`;
  });
}
