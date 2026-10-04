// JUAN (HeimVision/Zosi K9604-W family) view: streaming scan, findings, export.
// Standalone page: served at /juan.html (the main UI is left unchanged).

const PAGE = 50;
let root = null;
let polling = null;
let slotState = "recorded";
let slotOffset = 0;

const esc = (v) =>
  String(v ?? "—").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" }[c]));
const q = (sel) => root.querySelector(sel);
const fmtTime = (t) => (t ? esc(t.replace("T", " ").replace("Z", " UTC")) : "—");
const row = (k, v) => `<tr><th>${esc(k)}</th><td>${v}</td></tr>`;

async function call(method, path, body) {
  const r = await fetch(path, {
    method,
    headers: body ? { "Content-Type": "application/json" } : undefined,
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await r.text();
  if (!r.ok) throw new Error(text || r.statusText);
  try { return JSON.parse(text); } catch { return text; }
}

function remember(key, value) {
  try { if (value === undefined) return localStorage.getItem(key); localStorage.setItem(key, value); } catch { return null; }
  return null;
}

export function initJuan(container) {
  root = container;
  if (!root) return;
  const guess = document.getElementById("image-path")?.value || remember("satya.juan.path") || "";
  root.innerHTML = `
    <div class="juan">
      <section class="card">
        <h3>JUAN / HeimVision NVR</h3>
        <p class="muted">Streams the E01 or raw image; nothing is loaded into RAM or written to the evidence.</p>
        <label>Image path (.E01 or raw)</label>
        <input id="j-path" type="text" value="${esc(guess === "—" ? "" : guess)}" placeholder="/sdcard/Download/.../HeimVision K9604-W.E01">
        <div class="row">
          <label class="chk"><input id="j-deep" type="checkbox"> Deep (read empty slots fully)</label>
          <label class="chk"><input id="j-hash" type="checkbox"> Hash each payload</label>
        </div>
        <button id="j-scan" class="primary">Scan</button>
        <div id="j-progress" class="progress hidden"><div></div></div>
        <p id="j-msg" class="muted"></p>
      </section>
      <section id="j-results"></section>
      <section id="j-slots" class="card hidden"></section>
      <section id="j-export" class="card hidden">
        <h3>Export</h3>
        <div class="row">
          <label>From slot <input id="j-from" type="number" min="0" placeholder="first"></label>
          <label>To slot <input id="j-to" type="number" min="0" placeholder="last"></label>
          <label>FPS <input id="j-fps" type="number" step="0.1" min="1" max="120" placeholder="auto"></label>
        </div>
        <div class="row">
          <label class="chk"><input id="j-strip" type="checkbox" checked> Strip non-standard units</label>
          <label class="chk"><input id="j-res" type="checkbox"> Include residual (recovered) slots</label>
        </div>
        <button id="j-do-export" class="primary">Export H.265 + manifest</button>
        <div id="j-export-out"></div>
      </section>
    </div>`;
  q("#j-scan").addEventListener("click", startScan);
  q("#j-do-export").addEventListener("click", startExport);
  refresh();
}

async function startScan() {
  const image_path = q("#j-path").value.trim();
  if (!image_path) return;
  remember("satya.juan.path", image_path);
  try {
    await call("POST", "/api/juan/scan", { image_path, deep: q("#j-deep").checked, hash: q("#j-hash").checked });
    slotOffset = 0;
    poll();
  } catch (e) {
    msg("Scan failed: " + e.message, true);
  }
}

async function startExport() {
  const num = (id) => (q(id).value === "" ? undefined : Number(q(id).value));
  const body = { from: num("#j-from"), to: num("#j-to"), fps: num("#j-fps"), strip: q("#j-strip").checked, residual: q("#j-res").checked };
  q("#j-do-export").disabled = true;
  msg("Exporting… (reads every selected slot from the image)");
  poll();
  try {
    const r = await call("POST", "/api/juan/export", body);
    renderExport(r);
    msg("Export finished.");
  } catch (e) {
    msg("Export failed: " + e.message, true);
  } finally {
    q("#j-do-export").disabled = false;
  }
}

function poll() {
  if (polling) return;
  polling = setInterval(async () => {
    const s = await refresh();
    if (!s || !["scanning", "exporting"].includes(s.job.status)) {
      clearInterval(polling);
      polling = null;
    }
  }, 1000);
}

function msg(text, isError = false) {
  const el = q("#j-msg");
  el.textContent = text;
  el.className = isError ? "error" : "muted";
}

async function refresh() {
  let s;
  try { s = await call("GET", "/api/juan/status"); } catch { return null; }
  const job = s.job;
  const bar = q("#j-progress");
  if (job.status === "scanning") {
    bar.classList.remove("hidden");
    const pct = job.total ? Math.round((100 * job.done) / job.total) : 0;
    bar.firstElementChild.style.width = pct + "%";
    msg(`Scanning slots: ${job.done} / ${job.total || "?"} (${pct}%)`);
  } else {
    bar.classList.add("hidden");
  }
  if (job.status === "error") msg("Error: " + job.error, true);
  if (job.status === "done" && s.summary) {
    if (!polling || job.export === null) msg(`Scan complete: ${job.image_path}`);
    renderResults(s);
    q("#j-export").classList.remove("hidden");
    q("#j-slots").classList.remove("hidden");
    loadSlots();
    if (job.export) renderExport(job.export);
  }
  return s;
}

function hist(obj, label) {
  const entries = Object.entries(obj || {});
  if (!entries.length) return "—";
  return entries
    .sort((a, b) => b[1] - a[1])
    .slice(0, 8)
    .map(([k, v]) => `${esc(k)}${label} ×${esc(v)}`)
    .join("<br>");
}

function renderResults(s) {
  const { job, layout: l, summary: m, warnings } = s;
  const dirs = (m.dirs || [])
    .filter((d) => d.recorded > 0)
    .map((d) => `<tr><td>dir${String(d.dir).padStart(5, "0")}</td><td>${d.recorded}/${d.slots}</td><td>${fmtTime(d.first_start_utc)}</td><td>${fmtTime(d.last_end_utc)}</td></tr>`)
    .join("");
  const gaps = (m.gaps || []).slice(0, 10).map(([slot, sec]) => `slot ${slot}: ${sec}s`).join(", ") || "none";
  q("#j-results").innerHTML = `
    <section class="card">
      <h3>Evidence</h3>
      <table class="kv">
        ${row("Image", esc(job.image_path))}
        ${row("Media size", job.media_bytes ? (job.media_bytes / 1e9).toFixed(2) + " GB" : "—")}
        ${row("Stored MD5", `<code>${esc(job.stored_md5)}</code>`)}
        ${row("Stored SHA-1", `<code>${esc(job.stored_sha1)}</code>`)}
        ${row("Scan", `${fmtTime(job.started_utc)} → ${fmtTime(job.finished_utc)}`)}
      </table>
    </section>
    <section class="card">
      <h3>Layout (confidence ${(l.confidence * 100).toFixed(0)}%)</h3>
      <ul>${l.signals.map((x) => `<li>${esc(x)}</li>`).join("")}</ul>
      <table class="kv">
        ${row("FAT32 partition", `LBA ${l.fat_partition.start_lba} – ${l.fat_partition.end_lba}`)}
        ${row("System partition", l.ext_partition ? `${esc(l.ext_partition.fs)} at LBA ${l.ext_partition.start_lba}` : "—")}
        ${row("Cluster size", l.cluster_size + " bytes")}
        ${row("Deleted dir entries", l.deleted_dir_entries)}
      </table>
    </section>
    ${warnings.length ? `<section class="card warn"><h3>Check before reporting</h3><ul>${warnings.map((w) => `<li>${esc(w)}</li>`).join("")}</ul></section>` : ""}
    <section class="card">
      <h3>Findings</h3>
      <table class="kv">
        ${row("Slots", `${m.total_slots} total · ${m.recorded} recorded · ${m.empty} empty · ${m.residual} residual · ${m.unrecognized} unrecognized`)}
        ${row("FAT-dated slots", m.fat_dated_slots)}
        ${row("Header time span", `${fmtTime(m.first_start_utc)} → ${fmtTime(m.last_end_utc)}`)}
        ${row("Recorded seconds", m.recorded_seconds.toLocaleString())}
        ${row("Slot order monotonic", m.slot_order_monotonic ? "yes" : "<b>no</b>")}
        ${row("Overlaps / gaps > 5 s", `${m.overlaps} / ${gaps}`)}
        ${row("H.265 pictures", m.pictures.toLocaleString())}
        ${row("Est. FPS (all streams)", m.est_fps ? m.est_fps.toFixed(2) : "—")}
        ${row("Distinct SPS", m.distinct_sps)}
        ${row("NAL units / non-standard", `${m.nal_total.toLocaleString()} / ${m.nal_nonstandard.toLocaleString()}`)}
        ${row("Payload bytes", (m.payload_bytes / 1e6).toFixed(1) + " MB")}
        ${row("Header length (bytes)", hist(m.header_len_hist, ""))}
        ${row("FAT time − header end", hist(m.fat_delta_hist, " s"))}
      </table>
    </section>
    ${dirs ? `<section class="card"><h3>Directories with recordings</h3><div class="scroll"><table class="grid"><thead><tr><th>Dir</th><th>Rec/All</th><th>First start</th><th>Last end</th></tr></thead><tbody>${dirs}</tbody></table></div></section>` : ""}`;
}

async function loadSlots() {
  const el = q("#j-slots");
  let data;
  try { data = await call("GET", `/api/juan/slots?state=${slotState}&offset=${slotOffset}&limit=${PAGE}`); } catch { return; }
  const rows = data.slots
    .map((r) => `<tr>
      <td>${r.slot}</td><td>${esc(r.path)}</td><td>${esc(r.state)}</td>
      <td>${fmtTime(r.header_start_utc)}</td><td>${fmtTime(r.header_end_utc)}</td>
      <td>${esc(r.fat_created)}</td><td>${r.payload_offset ?? "—"}</td>
      <td>${(r.payload_len / 1e6).toFixed(2)} MB</td><td>${r.nal ? r.nal.pictures : "—"}</td>
      <td>${r.payload_extents && r.payload_extents[0] ? r.payload_extents[0].disk_offset : "—"}</td></tr>`)
    .join("");
  const last = Math.min(slotOffset + PAGE, data.total);
  el.innerHTML = `
    <h3>Slots</h3>
    <div class="row">
      <select id="j-state">
        ${["recorded", "data", "residual", "unrecognized", "empty", "all"].map((s) => `<option ${s === slotState ? "selected" : ""}>${s}</option>`).join("")}
      </select>
      <span class="muted">${data.total ? slotOffset + 1 : 0}–${last} of ${data.total}</span>
      <button id="j-prev" ${slotOffset === 0 ? "disabled" : ""}>‹</button>
      <button id="j-next" ${last >= data.total ? "disabled" : ""}>›</button>
    </div>
    <div class="scroll"><table class="grid">
      <thead><tr><th>#</th><th>Path</th><th>State</th><th>Header start</th><th>Header end</th><th>FAT created (local)</th><th>Hdr len</th><th>Payload</th><th>Pics</th><th>Disk offset</th></tr></thead>
      <tbody>${rows}</tbody></table></div>`;
  q("#j-state").addEventListener("change", (e) => { slotState = e.target.value; slotOffset = 0; loadSlots(); });
  q("#j-prev").addEventListener("click", () => { slotOffset = Math.max(0, slotOffset - PAGE); loadSlots(); });
  q("#j-next").addEventListener("click", () => { slotOffset += PAGE; loadSlots(); });
}

function renderExport(r) {
  const rep = r.report;
  const video = r.mp4_path
    ? `<video controls playsinline preload="metadata" src="/api/video?path=${encodeURIComponent(r.mp4_path)}"></video>`
    : `<p class="error">MP4 not created: ${esc(r.mp4_error)}. The .hevc stream is still valid evidence output (install ffmpeg to wrap it).</p>`;
  q("#j-export-out").innerHTML = `
    <table class="kv">
      ${row("Slots written", rep.slots_written)}
      ${row("Bytes", rep.bytes_written.toLocaleString())}
      ${row("NAL kept / dropped", `${rep.nal_kept} / ${rep.nal_dropped}`)}
      ${row("MD5", `<code>${esc(rep.md5)}</code>`)}
      ${row("SHA-256", `<code>${esc(rep.sha256)}</code>`)}
      ${row("H.265 file", `<code>${esc(r.hevc_path)}</code>`)}
      ${row("Manifest", `<code>${esc(r.manifest_path)}</code>`)}
      ${row("MP4 playback FPS", `${r.fps_used} (estimate, not evidence)`)}
    </table>
    ${video}`;
}
