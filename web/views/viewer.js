export function renderViewer(state) {
  const v = document.getElementById("video");
  const meta = document.getElementById("viewer-meta");
  if (!v) return;

  // Only reload if the source is empty or hasn't been set
  const needsLoad = !v.src || v.src === "" || v.src === window.location.href
    || v.dataset.loaded !== "1";

  if (needsLoad) {
    v.src = "/api/video?t=" + Date.now();
    v.dataset.loaded = "1";
    v.load();
  }

  meta.innerHTML = state.case
    ? `OEM: ${state.case.oem} · ${state.case.total_frames} frames · sha256 ${state.case.sha256.slice(0, 16)}…`
    : "No case loaded";

  v.onerror = () => {
    meta.textContent = "video error — export MP4 in the top bar or check the API is running";
    v.dataset.loaded = "";
  };
}

export function reloadVideo() {
  const v = document.getElementById("video");
  if (!v) return;
  v.src = "/api/video?t=" + Date.now();
  v.dataset.loaded = "1";
  v.load();
}
