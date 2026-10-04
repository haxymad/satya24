const $ = (id) => document.getElementById(id);

export function renderDetails(state) {
  if (!$("hex-pre")) return; // details pane removed from the layout
  const sel = state.selectedFrame;
  if (!sel) {
    $("hex-pre").textContent = "no frame selected";
    return;
  }
  const { idx, frame, timeline } = sel;

  $("detail-title").textContent = `Frame #${idx}`;

  $("detail-meta").innerHTML = `
    <dl>
      <dt>Offset</dt><dd>0x${frame.offset.toString(16)} (${frame.offset} bytes)</dd>
      <dt>Length</dt><dd>${frame.length} bytes</dd>
      <dt>Codec</dt><dd>${frame.codec}</dd>
      <dt>Recovery source</dt><dd>${frame.recovery_source}</dd>
      <dt>Recovery confidence</dt><dd>${frame.recovery_confidence}</dd>
      ${timeline ? `
        <dt>Posterior mean</dt><dd>${new Date(timeline.mean_s * 1000).toISOString()}</dd>
        <dt>Sigma</dt><dd>${timeline.sigma_s.toFixed(3)} s</dd>
        <dt>95% CI</dt>
        <dd>${new Date(timeline.ci_lo_95_s * 1000).toISOString()}<br>
            ${new Date(timeline.ci_hi_95_s * 1000).toISOString()}</dd>
        <dt>Collapsed</dt><dd>${timeline.collapsed}</dd>
        <dt>Sources</dt><dd>${(timeline.effective_sources || []).join(", ")}</dd>
      ` : ""}
    </dl>`;

  const claims = frame.claims || [];
  $("detail-claims").innerHTML = claims.length
    ? `<table><thead><tr><th>Source</th><th>Claimed (UTC)</th><th>Conf</th></tr></thead>
       <tbody>${claims.map((c) => `<tr>
         <td>${c.source}</td><td>${c.claimed_utc}</td>
         <td class="num">${c.confidence.toFixed(2)}</td>
       </tr>`).join("")}</tbody></table>`
    : `<p class="muted">No timestamp claims for this frame.</p>`;

  // Hex — direct offset lookup bypasses group indexing entirely
  const hexEl = $("hex-pre");
  hexEl.textContent = "loading…";
  const offset = frame.offset;
  const length = Math.min(frame.length || 512, 2048);

  fetch(`/api/image/hex?path=/frames/allocated/0&max_bytes=64`)
    .then(() => null)  // just probe the endpoint
    .catch(() => null);

  // Use a direct offset endpoint instead
  fetch(`/api/hex-at?offset=${offset}&length=${length}`)
    .then((r) => r.ok ? r.json() : r.text().then((t) => { throw new Error(t); }))
    .then((data) => {
      if (data.hex) hexEl.textContent = data.hex;
      else hexEl.textContent = "(empty)";
    })
    .catch((e) => { hexEl.textContent = `hex error: ${e.message}`; });
}
