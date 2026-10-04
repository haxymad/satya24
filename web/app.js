import { api } from "./api.js";
import { renderTree } from "./views/tree.js";
import { renderFrames } from "./views/frames.js";
import { renderTimeline } from "./views/timeline.js";
import { renderViewer, reloadVideo } from "./views/viewer.js";
import { renderML } from "./views/ml.js";
import { renderMCP } from "./views/mcp.js";
import { renderCustody } from "./views/custody.js";
import { renderDetails } from "./views/details.js";
import { renderDevice } from "./views/device.js";
import { renderChannels } from "./views/channels.js";
import { renderLogs } from "./views/logs.js";
import { renderAllocation } from "./views/allocation.js";
import { initFiles } from "./views/files.js";
import { initExplorers, refreshImageExplorer } from "./views/explorer.js";
import { initMenu, toast } from "./menu.js";
import { initPersons } from "./views/persons.js";

const $ = (id) => document.getElementById(id);
export const state = {
  case: null, frames: [], timeline: [], selectedFrame: null,
  search: "", filterSource: "all",
};

function setStatus(msg, isError = false) {
  $("status").textContent = msg;
  $("status").className = isError ? "error" : "";
}
function setCaseBar(info) {
  if (!info || !info.oem) {
    $("case-name").textContent = "[no case loaded]";
    $("status-right").textContent = "";
    return;
  }
  $("case-name").textContent =
    `${info.oem.toUpperCase()} · ${info.total_frames} frames ` +
    `(${info.allocated} alloc / ${info.carved} carved) · ` +
    `sha256 ${info.sha256.slice(0, 12)}…`;
  $("status-right").textContent =
    `${(info.size_bytes / 1e6).toFixed(1)} MB · md5 ${info.md5.slice(0, 12)}…`;
}

async function doAnalyze() {
  const path = $("image-path").value.trim();
  if (!path) return;
  setStatus("reading image…");
  $("btn-analyze").disabled = true;
  try {
    const info = await api.analyze(path);
    state.case = info;
    setCaseBar(info);
    setStatus("loading frames…");
    const [frames, timeline] = await Promise.all([api.frames(), api.timeline()]);
    state.frames = frames;
    state.timeline = timeline;
    renderTree(state);
    renderFrames(state);
    renderTimeline(state);
    renderCustody(state);
    renderMCP(state);
    renderML(state);
    renderDevice(state);
    renderChannels(state);
    renderLogs(state);
    renderAllocation(state);
    $("btn-export").disabled = false;
    $("btn-report").disabled = false;
    setStatus(`analyzed: ${frames.length} frames · video auto-exported`);
    $("search-count").textContent = `${frames.length} items`;

    // Auto-load the video so Viewer tab works without clicking Export
    reloadVideo();
    refreshImageExplorer();
  } catch (e) {
    setStatus("error: " + e.message, true);
  } finally {
    $("btn-analyze").disabled = false;
  }
}

async function doExport() {
  setStatus("extracting NAL units + wrapping with FFmpeg…");
  try {
    const r = await api.exportVideo();
    setStatus(`exported ${(r.size_bytes / 1e6).toFixed(1)} MB → ${r.mp4}`);
    document.querySelector('[data-view="viewer"]').click();
    reloadVideo();
    refreshImageExplorer();
  } catch (e) {
    setStatus("export error: " + e.message, true);
  }
}

async function doReport() {
  setStatus("generating PDF…");
  try {
    const val = (id) => document.getElementById(id)?.value?.trim() || "";
    const thumbs = parseInt(val("rf-thumbs"), 10);
    setStatus("generating PDF… (decoding evidence frames, this can take a minute)");
    const r = await api.genReport({
      case_id: val("rf-case-id"), case_name: val("rf-case-name"), examiner: val("rf-examiner"),
      organization: val("rf-org"), notes: val("rf-notes"), thumbnails: isNaN(thumbs) ? null : thumbs,
    });
    toast(`Report saved: ${r.output_pdf.split("/").pop()}`);
    setStatus(`PDF written: ${r.output_pdf}`);
    $("report-status").textContent = r.output_pdf;
  } catch (e) {
    setStatus("report error: " + e.message, true);
  }
}

document.querySelectorAll("#center-tabs .tab").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll("#center-tabs .tab").forEach((b) => b.classList.remove("active"));
    document.querySelectorAll("#pane-center .view").forEach((v) => v.classList.remove("active"));
    btn.classList.add("active");
    document.getElementById("view-" + btn.dataset.view).classList.add("active");
    const v = btn.dataset.view;
    if (v === "timeline") renderTimeline(state);
    if (v === "viewer") renderViewer(state);
    if (v === "device") renderDevice(state);
    if (v === "channels") renderChannels(state);
    if (v === "logs") renderLogs(state);
    if (v === "allocation") renderAllocation(state);
    if (v === "xplore") refreshImageExplorer();
    if (v === "mcp") renderMCP(state);
    if (v === "custody") renderCustody(state);
  });
});

document.querySelectorAll(".dt").forEach((btn) => {
  btn.addEventListener("click", () => {
    document.querySelectorAll(".dt").forEach((b) => b.classList.remove("active"));
    document.querySelectorAll(".detail-view").forEach((v) => v.classList.remove("active"));
    btn.classList.add("active");
    document.getElementById("detail-" + btn.dataset.dt).classList.add("active");
  });
});

$("search").addEventListener("input", (e) => { state.search = e.target.value.toLowerCase(); renderFrames(state); });
$("filter-source").addEventListener("change", (e) => { state.filterSource = e.target.value; renderFrames(state); });
$("btn-analyze").addEventListener("click", doAnalyze);
$("btn-export").addEventListener("click", doExport);
$("btn-report").addEventListener("click", doReport);
$("btn-gen-report").addEventListener("click", doReport);
$("image-path").addEventListener("keydown", (e) => { if (e.key === "Enter") doAnalyze(); });

document.addEventListener("frame-selected", (e) => {
  state.selectedFrame = e.detail;
  renderDetails(state);
  renderTimeline(state);
});
document.addEventListener("frame-filter-changed", () => renderFrames(state));
document.addEventListener("video-loaded", () => {
  renderViewer(state);
  reloadVideo();
});

initFiles();
initMenu();
initExplorers();
initPersons();
renderML(state);
setStatus("ready — pick an image from the Files panel or type a path");
