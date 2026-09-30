const $ = (id) => document.getElementById(id);
let currentPath = "/";

export function initImageFs() {
  const go = $("ifs-go");
  const up = $("ifs-up");
  const ref = $("ifs-refresh");
  const inp = $("ifs-path");
  if (go) go.addEventListener("click", () => load(inp.value || "/"));
  if (inp) inp.addEventListener("keydown", (e) => {
    if (e.key === "Enter") load(inp.value || "/");
  });
  if (up) up.addEventListener("click", () => {
    const parts = currentPath.split("/").filter(Boolean);
    parts.pop();
    load("/" + parts.join("/"));
  });
  if (ref) ref.addEventListener("click", () => load(currentPath));
}

export async function loadImageRoot() {
  currentPath = "/";
  await load("/");
}

async function load(path) {
  const tree = $("ifs-tree");
  const preview = $("ifs-preview");
  if (!tree) return;
  tree.innerHTML = `<div class="fs-empty">loading…</div>`;
  try {
    const r = await fetch(`/api/image/list?path=${encodeURIComponent(path)}`);
    if (!r.ok) throw new Error(await r.text());
    const data = await r.json();
    currentPath = data.path || "/";
    $("ifs-path").value = currentPath;
    renderTree(data.nodes);
    preview.innerHTML = `<div class="fs-empty">select a file to view</div>`;
  } catch (e) {
    tree.innerHTML = `<div class="fs-empty">${escapeHtml(e.message)}</div>`;
  }
}

function renderTree(nodes) {
  const tree = $("ifs-tree");
  if (!nodes.length) {
    tree.innerHTML = `<div class="fs-empty">(empty — click ↑ to go back)</div>`;
    return;
  }
  tree.innerHTML = nodes.map((n) => `
    <div class="ifs-node ifs-${n.kind}" data-path="${escapeAttr(n.path)}" data-kind="${n.kind}">
      <span class="ifs-icon">${n.kind === "dir" ? "▸" : (n.kind === "hex" ? "◉" : "·")}</span>
      <span class="ifs-name">${escapeHtml(n.name)}</span>
      <span class="ifs-detail">${escapeHtml(n.detail || "")}</span>
    </div>
  `).join("");

  tree.querySelectorAll(".ifs-node").forEach((row) => {
    row.addEventListener("click", () => onClick(row));
  });
}

async function onClick(row) {
  const path = row.dataset.path;
  const kind = row.dataset.kind;
  if (kind === "dir") { load(path); return; }

  row.parentElement.querySelectorAll(".ifs-node").forEach((r) => r.classList.remove("active"));
  row.classList.add("active");
  await showPreview(path);
}

async function showPreview(path) {
  const el = $("ifs-preview");
  el.innerHTML = `<div class="fs-empty">loading…</div>`;
  try {
    const [hexResp, nodeResp] = await Promise.all([
      fetch(`/api/image/hex?path=${encodeURIComponent(path)}&max_bytes=4096`),
      fetch(`/api/image/node?path=${encodeURIComponent(path)}`),
    ]);
    if (!hexResp.ok) throw new Error(await hexResp.text());
    const hex = await hexResp.json();

    let detail = null;
    if (nodeResp.ok) {
      try { detail = await nodeResp.json(); } catch {}
    }

    el.innerHTML = `
      <div class="ifs-preview-header">
        <b>${escapeHtml(path)}</b>
        <span class="muted">0x${hex.start.toString(16)} .. 0x${hex.end.toString(16)} · ${hex.size} bytes</span>
      </div>
      ${detail ? `<div class="ifs-preview-detail">
        <div class="ifs-detail-label">Parsed</div>
        <pre>${escapeHtml(JSON.stringify(detail, null, 2))}</pre>
      </div>` : ""}
      <div class="ifs-preview-hex">
        <div class="ifs-detail-label">Hex</div>
        <pre>${escapeHtml(hex.hex)}</pre>
      </div>
    `;
  } catch (e) {
    el.innerHTML = `<div class="fs-empty">${escapeHtml(e.message)}</div>`;
  }
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[c]);
}
function escapeAttr(s) { return escapeHtml(s).replace(/"/g, "&quot;"); }
