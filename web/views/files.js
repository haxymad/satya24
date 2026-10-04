const $ = (id) => document.getElementById(id);

let currentPath = "";
// Disk images the Analyze button accepts (E01 = first segment of an EnCase/FTK image).
const IMAGE_EXTS = ["img", "dd", "raw", "e01"];

export async function initFiles() {
  const roots = await fetch("/api/fs/roots").then((r) => r.json());
  currentPath = roots.home || "/";
  $("fs-path").value = currentPath;
  await load(currentPath);

  $("fs-go").addEventListener("click", () => {
    const p = $("fs-path").value.trim();
    if (p) load(p);
  });
  $("fs-path").addEventListener("keydown", (e) => {
    if (e.key === "Enter") {
      const p = $("fs-path").value.trim();
      if (p) load(p);
    }
  });
}

async function load(path) {
  const el = $("fs-list");
  el.innerHTML = `<div class="fs-empty">loading…</div>`;
  try {
    const r = await fetch(`/api/fs/list?path=${encodeURIComponent(path)}`);
    if (!r.ok) throw new Error(await r.text());
    const data = await r.json();
    currentPath = data.path;
    $("fs-path").value = currentPath;
    render(data);
  } catch (e) {
    el.innerHTML = `<div class="fs-empty">${e.message}</div>`;
  }
}

function fmtSize(bytes) {
  if (bytes === 0) return "";
  const u = ["B", "K", "M", "G", "T"];
  let i = 0, b = bytes;
  while (b >= 1024 && i < u.length - 1) { b /= 1024; i++; }
  return b.toFixed(b < 10 ? 1 : 0) + u[i];
}

function classFor(entry) {
  if (entry.is_dir) return "dir";
  if (IMAGE_EXTS.includes(entry.ext)) return "img";
  if (["mp4", "h264", "mkv", "webm", "avi"].includes(entry.ext)) return "vid";
  return "";
}

function render(data) {
  const el = $("fs-list");
  let html = "";
  if (data.parent) {
    html += `<div class="fs-entry dir" data-path="${data.parent}" data-type="dir"><span class="nm">..</span></div>`;
  }
  if (!data.entries.length) {
    html += `<div class="fs-empty">empty</div>`;
  }
  for (const e of data.entries) {
    const cls = classFor(e);
    const icon = e.is_dir ? "▸" : (cls === "img" ? "◉" : (cls === "vid" ? "▶" : "·"));
    html += `<div class="fs-entry ${cls}" data-path="${escapeAttr(e.path)}" data-type="${e.is_dir ? "dir" : "file"}" data-ext="${e.ext}">
      <span class="nm">${icon} ${escapeHtml(e.name)}</span>
      <span class="sz">${fmtSize(e.size_bytes)}</span>
    </div>`;
  }
  el.innerHTML = html;

  el.querySelectorAll(".fs-entry").forEach((row) => {
    row.addEventListener("click", () => onEntry(row));
  });
}

function onEntry(row) {
  const path = row.dataset.path;
  const type = row.dataset.type;
  const ext = row.dataset.ext;

  if (type === "dir") {
    load(path);
    return;
  }

  if (IMAGE_EXTS.includes(ext)) {
    $("image-path").value = path;
    document.getElementById("status").textContent = `selected image: ${path}`;
    return;
  }

  if (["mp4", "h264", "mkv", "webm", "avi"].includes(ext)) {
    loadVideo(path);
    return;
  }

  document.getElementById("status").textContent = `file: ${path}`;
}

async function loadVideo(path) {
  try {
    const r = await fetch("/api/load-video", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ path }),
    });
    if (!r.ok) throw new Error(await r.text());
    document.getElementById("status").textContent = `loaded video: ${path}`;
    document.querySelector('[data-view="viewer"]').click();
    // Trigger viewer refresh
    document.dispatchEvent(new CustomEvent("video-loaded"));
  } catch (e) {
    document.getElementById("status").textContent = `video error: ${e.message}`;
  }
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[c]);
}
function escapeAttr(s) {
  return escapeHtml(s).replace(/"/g, "&quot;");
}
