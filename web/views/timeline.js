export function renderTimeline(state) {
  const canvas = document.getElementById("timeline-canvas");
  if (!canvas || !state.timeline.length) return;
  const dpr = window.devicePixelRatio || 1;
  const rect = canvas.getBoundingClientRect();
  canvas.width = rect.width * dpr;
  canvas.height = rect.height * dpr;
  const ctx = canvas.getContext("2d");
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  const tl = state.timeline;
  const W = rect.width, H = rect.height;
  const PAD = { l: 90, r: 24, t: 24, b: 44 };
  const innerW = W - PAD.l - PAD.r, innerH = H - PAD.t - PAD.b;
  const means = tl.map((t) => t.mean_s);
  const los = tl.map((t) => t.ci_lo_95_s);
  const his = tl.map((t) => t.ci_hi_95_s);
  const tMin = Math.min(...los), tMax = Math.max(...his);
  const range = Math.max(1, tMax - tMin);
  const x = (i) => PAD.l + (i / Math.max(1, tl.length - 1)) * innerW;
  const y = (t) => PAD.t + (1 - (t - tMin) / range) * innerH;
  ctx.fillStyle = "#252526"; ctx.fillRect(0, 0, W, H);
  ctx.strokeStyle = "#3c3c3c"; ctx.lineWidth = 1;
  ctx.font = "10px ui-monospace, monospace";
  for (let i = 0; i <= 8; i++) {
    const yy = PAD.t + (i / 8) * innerH;
    ctx.beginPath(); ctx.moveTo(PAD.l, yy); ctx.lineTo(W - PAD.r, yy); ctx.stroke();
    ctx.fillStyle = "#858585"; ctx.textAlign = "right";
    const tv = tMax - (i / 8) * range;
    ctx.fillText(new Date(tv * 1000).toISOString().slice(11, 19), PAD.l - 8, yy + 3);
  }
  ctx.textAlign = "center"; ctx.fillStyle = "#858585";
  const labelStep = Math.max(1, Math.ceil(tl.length / 10));
  for (let i = 0; i < tl.length; i += labelStep) ctx.fillText(String(i), x(i), H - PAD.b + 16);
  ctx.fillText("frame index", W / 2, H - 6);
  if (document.getElementById("tl-show-ci").checked) {
    ctx.fillStyle = "rgba(79, 157, 255, 0.25)";
    ctx.beginPath();
    ctx.moveTo(x(0), y(his[0]));
    for (let i = 1; i < tl.length; i++) ctx.lineTo(x(i), y(his[i]));
    for (let i = tl.length - 1; i >= 0; i--) ctx.lineTo(x(i), y(los[i]));
    ctx.closePath(); ctx.fill();
  }
  ctx.strokeStyle = "#e94560"; ctx.lineWidth = 1.5;
  ctx.beginPath(); ctx.moveTo(x(0), y(means[0]));
  for (let i = 1; i < tl.length; i++) ctx.lineTo(x(i), y(means[i]));
  ctx.stroke();
  if (state.selectedFrame && state.selectedFrame.idx < tl.length) {
    const sel = state.selectedFrame.idx;
    ctx.fillStyle = "#4f9dff";
    ctx.beginPath(); ctx.arc(x(sel), y(means[sel]), 5, 0, Math.PI * 2); ctx.fill();
  }
  document.getElementById("timeline-summary").textContent =
    `${tl.length} frames • span ${(range / 60).toFixed(1)} min • ` +
    `mean σ ${(tl.reduce((s, t) => s + t.sigma_s, 0) / tl.length).toFixed(2)} s`;
  canvas.onclick = (e) => {
    const rect = canvas.getBoundingClientRect();
    const px = e.clientX - rect.left;
    if (px < PAD.l || px > W - PAD.r) return;
    const idx = Math.round(((px - PAD.l) / innerW) * (tl.length - 1));
    if (idx < 0 || idx >= tl.length) return;
    document.dispatchEvent(new CustomEvent("frame-selected", {
      detail: { idx, frame: state.frames[idx], timeline: tl[idx] },
    }));
  };
}
