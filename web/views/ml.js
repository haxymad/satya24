const $ = (id) => document.getElementById(id);

async function postJSON(path, body = {}) {
  const r = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  const text = await r.text();
  if (!r.ok) throw new Error(text);
  try { return JSON.parse(text); } catch { return { raw: text }; }
}

export function renderML(_state) {
  $("btn-ml-run").onclick = () => run("/api/ml/detect", { frame_index: parseInt($("ml-frame").value, 10) || 0 });
  $("btn-ml-faces").onclick = () => run("/api/ml/faces", { frame_index: parseInt($("ml-frame").value, 10) || 0 });
  $("btn-ml-motion").onclick = () => run("/api/ml/motion", {
    start_frame: 0,
    end_frame: Math.min(500, (parseInt($("ml-frame").value, 10) || 100) + 200),
    step: 5,
    threshold: 25.0,
  });
  $("btn-ml-health").onclick = checkHealth;
}

async function checkHealth() {
  const s = $("ml-status");
  s.textContent = "checking…";
  try {
    const r = await fetch("/api/ml/health");
    const t = await r.text();
    if (!r.ok) {
      s.textContent = "ML offline";
      $("ml-results").innerHTML = `<div class="ml-error">
        <b>ML sidecar not reachable</b>
        <pre>${escapeHtml(t)}</pre>
        <p>Start it with:</p>
        <pre>cd python\npython3 -m dvr_forensics.ml_server</pre>
      </div>`;
      return;
    }
    const data = JSON.parse(t);
    s.textContent = "ML online";
    $("ml-results").innerHTML = `<pre>${escapeHtml(JSON.stringify(data, null, 2))}</pre>`;
  } catch (e) {
    s.textContent = "ML offline";
    $("ml-results").innerHTML = `<div class="ml-error">${escapeHtml(e.message)}</div>`;
  }
}

async function run(path, body) {
  const s = $("ml-status");
  s.textContent = "running…";
  $("ml-results").innerHTML = "";
  try {
    const data = await postJSON(path, body);
    s.textContent = "ok";
    $("ml-results").innerHTML = renderResult(path, data);
  } catch (e) {
    s.textContent = "error";
    $("ml-results").innerHTML = `<div class="ml-error">
      <b>${escapeHtml(e.message)}</b>
      <p>If ML sidecar is not running:</p>
      <pre>cd python && python3 -m dvr_forensics.ml_server</pre>
    </div>`;
  }
}

function renderResult(path, data) {
  if (path === "/api/ml/detect") {
    const dets = data.detections || [];
    if (!dets.length) {
      return `<p class="muted">No objects detected in frame ${data.frame_index ?? "?"}.</p>`;
    }
    return `<table>
      <thead><tr><th>Label</th><th>Score</th><th>BBox (x1, y1, x2, y2)</th></tr></thead>
      <tbody>${dets.map((d) => `<tr>
        <td>${escapeHtml(d.label)}</td>
        <td class="num">${d.score.toFixed(3)}</td>
        <td>${d.bbox.join(", ")}</td>
      </tr>`).join("")}</tbody></table>`;
  }
  if (path === "/api/ml/faces") {
    const fs = data.faces || [];
    if (!fs.length) {
      return `<p class="muted">${escapeHtml(data.note || "No faces detected.")}</p>`;
    }
    return `<table>
      <thead><tr><th>BBox</th><th>Score</th><th>Embedding</th></tr></thead>
      <tbody>${fs.map((f) => `<tr>
        <td>${f.bbox.join(", ")}</td>
        <td class="num">${f.score.toFixed(3)}</td>
        <td>${f.embedding ? "512-d" : "—"}</td>
      </tr>`).join("")}</tbody></table>`;
  }
  if (path === "/api/ml/motion") {
    const ev = data.events || [];
    return `<p class="muted">${ev.length} motion event(s) between frames ${data.start} and ${data.end}.</p>
      ${ev.length ? `<table><thead><tr><th>Frame</th><th>Motion %</th><th>Regions</th></tr></thead>
      <tbody>${ev.slice(0, 200).map((e) => `<tr>
        <td class="num">${e.frame_index}</td>
        <td class="num">${e.motion_pct}</td>
        <td>${e.regions.length}</td>
      </tr>`).join("")}</tbody></table>` : ""}`;
  }
  return `<pre>${escapeHtml(JSON.stringify(data, null, 2))}</pre>`;
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[c]);
}
