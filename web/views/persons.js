const $ = (id) => document.getElementById(id);

export function initPersons() {
  const btn = $("btn-persons-run");
  if (btn) btn.addEventListener("click", run);
}

async function run() {
  const status = $("persons-status");
  const results = $("persons-results");
  status.textContent = "running YOLO over sampled frames…";
  results.innerHTML = "";

  const sampleEvery = parseInt($("persons-sample").value, 10) || 25;
  const conf = parseFloat($("persons-conf").value) || 0.35;

  try {
    const r = await fetch("/api/ml/persons", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ sample_every: sampleEvery, conf }),
    });
    const text = await r.text();
    if (!r.ok) throw new Error(text);
    const data = JSON.parse(text);

    status.textContent =
      `${data.person_count} person detection(s) across ${data.sampled_frames} sampled frames`;

    if (!data.person_count) {
      results.innerHTML = `<p class="muted">No persons detected. Try lowering the confidence.</p>`;
      return;
    }

    results.innerHTML = `
      <div class="person-grid">
        ${data.persons.map((p, i) => `
          <div class="person-card" data-idx="${i}">
            <div class="person-img">
              ${p.crop_png_b64
                ? `<img src="data:image/png;base64,${p.crop_png_b64}" alt="person">`
                : `<div class="person-noimg">no crop</div>`}
            </div>
            <div class="person-meta">
              <div class="person-frame">frame ${p.frame_index}</div>
              <div class="person-time">${p.timestamp_s != null
                ? p.timestamp_s.toFixed(2) + " s"
                : "—"}</div>
              <div class="person-score">score ${p.score.toFixed(3)}</div>
              <div class="person-bbox">${p.bbox.join(", ")}</div>
            </div>
          </div>
        `).join("")}
      </div>
      <p class="muted" style="margin-top:16px">Click any card to open its frame in the Viewer.</p>
    `;

    results.querySelectorAll(".person-card").forEach((card) => {
      card.addEventListener("click", () => openFrame(data.persons[parseInt(card.dataset.idx, 10)]));
    });
  } catch (e) {
    status.textContent = "error";
    results.innerHTML = `<div class="ml-error">
      <b>${escapeHtml(e.message)}</b>
      <p>Ensure the ML sidecar is running:</p>
      <pre>cd python && python3 -m dvr_forensics.ml_server</pre>
    </div>`;
  }
}

function openFrame(p) {
  const v = document.getElementById("video");
  if (!v) return;
  document.querySelector('[data-view="viewer"]').click();
  v.pause();
  // Seek to the frame's timestamp
  if (p.timestamp_s != null) {
    try { v.currentTime = p.timestamp_s; } catch {}
  }
  v.play().catch(() => {});
  document.getElementById("status").textContent =
    `viewing frame ${p.frame_index} (score ${p.score.toFixed(3)})`;
}

function escapeHtml(s) {
  return String(s).replace(/[&<>"']/g, (c) => ({
    "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;",
  })[c]);
}
