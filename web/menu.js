// Menu bar, dialogs and notifications.
// Actions reuse the existing buttons where they exist (Analyze, Export MP4,
// Generate Report) so behaviour stays identical to clicking them.

const esc = (v) =>
  String(v ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const fmtSize = (b) => (b < 1024 ? `${b} B` : b < 1048576 ? `${(b / 1024).toFixed(1)} KiB` : b < 1073741824 ? `${(b / 1048576).toFixed(1)} MiB` : `${(b / 1073741824).toFixed(2)} GiB`);

async function call(method, url, body) {
  const r = await fetch(url, {
    method,
    headers: body ? { "Content-Type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await r.text();
  if (!r.ok) throw new Error(text || r.statusText);
  try { return JSON.parse(text); } catch { return text; }
}

// ---------------------------------------------------------------------------
// Toasts and dialogs
// ---------------------------------------------------------------------------

export function toast(msg, isError = false) {
  let box = document.getElementById("toasts");
  if (!box) {
    box = document.createElement("div");
    box.id = "toasts";
    document.body.appendChild(box);
  }
  const t = document.createElement("div");
  t.className = "toast" + (isError ? " err" : "");
  t.textContent = msg;
  box.appendChild(t);
  setTimeout(() => t.classList.add("out"), isError ? 7000 : 3500);
  setTimeout(() => t.remove(), isError ? 7600 : 4100);
}

export function openModal(title, html, { wide = false } = {}) {
  closeModal();
  const m = document.createElement("div");
  m.id = "modal";
  m.innerHTML = `<div class="modal-box${wide ? " wide" : ""}" role="dialog" aria-modal="true">
      <div class="modal-head"><span>${esc(title)}</span><button class="modal-x" title="Close (Esc)">✕</button></div>
      <div class="modal-body">${html}</div></div>`;
  m.onclick = (e) => { if (e.target === m) closeModal(); };
  m.querySelector(".modal-x").onclick = closeModal;
  document.body.appendChild(m);
  return m.querySelector(".modal-body");
}

export function closeModal() {
  document.getElementById("modal")?.remove();
}

const pathRow = (label, p) =>
  `<tr><th>${esc(label)}</th><td><code>${esc(p)}</code> <button class="mini" data-copy="${esc(p)}">copy</button></td></tr>`;
function bindCopy(root) {
  root.querySelectorAll("[data-copy]").forEach((b) => (b.onclick = () => navigator.clipboard?.writeText(b.dataset.copy).then(() => toast("Copied"))));
}

// ---------------------------------------------------------------------------
// Actions
// ---------------------------------------------------------------------------

const click = (id) => document.getElementById(id)?.click();
const tab = (view) => document.querySelector(`#center-tabs [data-view="${view}"]`)?.click();

async function saveCase() {
  try {
    const r = await call("POST", "/api/case/save");
    const body = openModal("Case saved", `<table class="xp-kv">${pathRow("File", r.path)}
      <tr><th>Size</th><td>${fmtSize(r.bytes)}</td></tr><tr><th>SHA-256</th><td><code>${esc(r.sha256)}</code></td></tr></table>
      <p class="muted">Includes the analysis results, the timeline and the signed activity log.</p>`);
    bindCopy(body);
  } catch (e) { toast(e.message, true); }
}

async function closeCase() {
  if (!confirm("Close the current case? Unsaved results are kept only in the output folder.")) return;
  await call("POST", "/api/case/close").catch(() => {});
  location.reload();
}

async function exportCsv() {
  try {
    const r = await call("POST", "/api/case/export-csv");
    bindCopy(openModal("Frame table exported", `<table class="xp-kv">${pathRow("File", r.path)}<tr><th>Rows</th><td>${r.rows}</td></tr></table>`));
  } catch (e) { toast(e.message, true); }
}

async function showOutputs() {
  try {
    const r = await call("GET", "/api/outputs");
    const rows = r.files.length
      ? r.files.map((f) => `<tr><td>${esc(f.name)}</td><td class="num">${fmtSize(f.bytes)}</td><td class="muted">${esc((f.modified || "").replace("T", " ").slice(0, 19))}</td>
          <td><button class="mini" data-copy="${esc(f.path)}">copy path</button>${/\.(mp4|pdf|jpg|png|csv|json|txt)$/i.test(f.name) ? ` <a class="mini" target="_blank" href="${f.name.endsWith(".mp4") ? "/api/video?path=" : "/api/local/raw?path="}${encodeURIComponent(f.path)}">open</a>` : ""}</td></tr>`).join("")
      : `<tr><td colspan="4" class="empty">Nothing exported yet</td></tr>`;
    bindCopy(openModal("Output folder", `<p class="muted">${esc(r.dir)}</p>
      <div class="modal-scroll"><table><thead><tr><th>File</th><th>Size</th><th>Modified (UTC)</th><th></th></tr></thead><tbody>${rows}</tbody></table></div>`, { wide: true }));
  } catch (e) { toast(e.message, true); }
}

let verifyTimer = null;
async function verify() {
  try {
    await call("POST", "/api/verify/start");
  } catch (e) {
    if (!/already running/.test(e.message)) return toast(e.message, true);
  }
  const body = openModal("Verify image integrity", `<p>Re-hashing the full logical media (MD5, SHA-1, SHA-256) and comparing it with the hashes stored at acquisition.</p>
    <div class="progress"><div class="progress-bar" style="width:0%"></div></div><p class="verify-msg muted">starting…</p>
    <p class="muted">You can close this window; verification continues in the background (Tools › Verify shows progress again).</p>`);
  clearInterval(verifyTimer);
  verifyTimer = setInterval(async () => {
    const j = await call("GET", "/api/verify/status").catch(() => null);
    const bar = document.querySelector("#modal .progress-bar");
    const msg = document.querySelector("#modal .verify-msg");
    if (!j) return;
    const pct = j.total ? (j.done * 100) / j.total : 0;
    if (bar) bar.style.width = `${pct.toFixed(1)}%`;
    if (j.status === "running" && msg) msg.textContent = `${fmtSize(j.done)} of ${fmtSize(j.total)} (${pct.toFixed(1)}%)`;
    if (j.status === "done" || j.status === "error") {
      clearInterval(verifyTimer);
      if (!document.getElementById("modal")) return toast(j.status === "done" ? "Verification finished" : "Verification failed", j.status !== "done");
      if (j.status === "error") { if (msg) msg.textContent = j.error; return; }
      const h = j.result;
      const v = (m) => (m === true ? `<span class="xp-badge ok">matches stored</span>` : m === false ? `<span class="xp-badge del">DOES NOT MATCH</span>` : `<span class="muted">no stored hash</span>`);
      body.innerHTML = `<table class="xp-kv">
        <tr><th>MD5</th><td><code>${esc(h.md5)}</code> ${v(h.md5_matches)}</td></tr>
        <tr><th>SHA-1</th><td><code>${esc(h.sha1)}</code> ${v(h.sha1_matches)}</td></tr>
        <tr><th>SHA-256</th><td><code>${esc(h.sha256)}</code></td></tr>
        <tr><th>Bytes</th><td>${h.bytes.toLocaleString()}</td></tr></table>
        <p class="muted">Logged in the activity log; the next report will include this result.</p>`;
    }
  }, 1000);
}

function shortcuts() {
  openModal("Keyboard shortcuts", `<table class="xp-kv">
    <tr><th>Ctrl+O</th><td>Open evidence (Local Disk)</td></tr>
    <tr><th>Ctrl+Enter</th><td>Analyze</td></tr>
    <tr><th>Ctrl+S</th><td>Save case</td></tr>
    <tr><th>Ctrl+Shift+E</th><td>Export video</td></tr>
    <tr><th>Ctrl+P</th><td>Forensic report</td></tr>
    <tr><th>Ctrl+B</th><td>Show / hide the side panel</td></tr>
    <tr><th>Esc</th><td>Close dialog or menu</td></tr>
    <tr><th>Backspace</th><td>Up one folder (in explorers)</td></tr></table>`);
}

function about() {
  openModal("About SATYA", `<p><b>SATYA</b> — multi-vendor DVR/NVR forensic analysis.</p>
    <p class="muted">Evidence images are opened read-only. Exports, extracted files and reports are written to the output folder with hashes, and every action is recorded in a signed, hash-chained activity log.</p>`);
}

function toggleSide() {
  document.getElementById("main")?.classList.toggle("no-side");
}

// ---------------------------------------------------------------------------
// Menu definition
// ---------------------------------------------------------------------------

const MENUS = [
  ["File", [
    ["Open evidence…", "Ctrl+O", () => tab("local")],
    ["Analyze", "Ctrl+Enter", () => click("btn-analyze")],
    null,
    ["Save case", "Ctrl+S", saveCase],
    ["Close case", "", closeCase],
  ]],
  ["Export", [
    ["Export video (all recordings)", "Ctrl+Shift+E", () => click("btn-export")],
    ["Export frame table (CSV)", "", exportCsv],
    ["Extract files from image…", "", () => tab("xplore")],
    null,
    ["Show output folder", "", showOutputs],
  ]],
  ["Report", [
    ["Forensic report (PDF)…", "Ctrl+P", () => tab("report")],
    ["Show output folder", "", showOutputs],
  ]],
  ["Tools", [
    ["Verify image integrity", "", verify],
    ["Image explorer", "", () => tab("xplore")],
    ["Local disk", "", () => tab("local")],
    null,
    ["AI provider & API key", "", () => tab("mcp")],
  ]],
  ["View", [
    ["Table", "", () => tab("table")],
    ["Timeline", "", () => tab("timeline")],
    ["Viewer", "", () => tab("viewer")],
    ["Device", "", () => tab("device")],
    ["Custody / activity log", "", () => tab("custody")],
    null,
    ["Show / hide side panel", "Ctrl+B", toggleSide],
  ]],
  ["Help", [
    ["Keyboard shortcuts", "", shortcuts],
    ["About SATYA", "", about],
  ]],
];

export function initMenu() {
  const bar = document.getElementById("menubar");
  if (!bar) return;
  bar.innerHTML = MENUS.map(([name, items], i) => `
    <div class="menu" data-i="${i}">
      <button class="menu-btn">${name}</button>
      <div class="menu-drop">${items.map((it, k) => it
        ? `<button class="menu-item" data-k="${k}"><span>${esc(it[0])}</span><span class="menu-key">${esc(it[1])}</span></button>`
        : `<div class="menu-sep"></div>`).join("")}</div>
    </div>`).join("");
  const closeAll = () => bar.querySelectorAll(".menu.open").forEach((m) => m.classList.remove("open"));
  bar.querySelectorAll(".menu").forEach((m) => {
    const btn = m.querySelector(".menu-btn");
    btn.onclick = (e) => {
      e.stopPropagation();
      const open = m.classList.contains("open");
      closeAll();
      if (!open) m.classList.add("open");
    };
    btn.onmouseenter = () => { if (bar.querySelector(".menu.open") && !m.classList.contains("open")) { closeAll(); m.classList.add("open"); } };
    m.querySelectorAll(".menu-item").forEach((it) => (it.onclick = (e) => {
      e.stopPropagation();
      closeAll();
      MENUS[+m.dataset.i][1][+it.dataset.k][2]();
    }));
  });
  document.addEventListener("click", closeAll);
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape") { closeAll(); closeModal(); return; }
    const ctrl = e.ctrlKey || e.metaKey;
    if (!ctrl) return;
    const k = e.key.toLowerCase();
    const run = (f) => { e.preventDefault(); f(); };
    if (k === "s") run(saveCase);
    else if (k === "o") run(() => tab("local"));
    else if (k === "p") run(() => tab("report"));
    else if (k === "b") run(toggleSide);
    else if (k === "enter") run(() => click("btn-analyze"));
    else if (k === "e" && e.shiftKey) run(() => click("btn-export"));
  });
}
