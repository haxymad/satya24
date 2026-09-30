import { api } from "../api.js";

export async function renderDevice(state) {
  const el = document.getElementById("device-panel");
  if (!el) return;
  try {
    const d = await api.device();
    el.innerHTML = `
      <h3 style="margin:0 0 14px;font-size:14px">Device Information</h3>
      <table>
        <tbody>
          ${row("OEM", d.oem)}
          ${row("Model", d.model || "—")}
          ${row("Firmware", d.firmware || "—")}
          ${row("Serial", d.serial || "—")}
          ${row("UUID", d.uuid || "—")}
          ${row("Manufacturer", d.manufacturer_string || "—")}
          ${row("Block size", d.block_size ? d.block_size + " bytes" : "—")}
          ${row("Sector size", d.sector_size ? d.sector_size + " bytes" : "—")}
          ${row("Image size", d.total_bytes.toLocaleString() + " bytes")}
          ${row("Disk size", d.disk_size_gb.toFixed(3) + " GB")}
        </tbody>
      </table>
      <h3 style="margin:22px 0 14px;font-size:14px">Video Statistics</h3>
      <div id="stats-panel"></div>
    `;
    const stats = await api.stats();
    document.getElementById("stats-panel").innerHTML = `
      <table><tbody>
        ${row("Total frames", stats.total_frames)}
        ${row("Allocated", stats.allocated)}
        ${row("Carved (deleted)", stats.carved)}
        ${row("Partial", stats.partial)}
        ${row("H.264 / H.265", stats.h264 + " / " + stats.h265)}
        ${row("Key frames", stats.key_frames)}
        ${row("Total video bytes", stats.total_video_bytes.toLocaleString())}
        ${row("Duration (est.)", stats.duration_seconds ? stats.duration_seconds.toFixed(1) + " s" : "—")}
        ${row("FPS (est.)", stats.fps_estimate || "—")}
        ${row("Bitrate (est.)", stats.bitrate_kbps ? stats.bitrate_kbps.toFixed(0) + " kbps" : "—")}
      </tbody></table>
    `;
  } catch (e) {
    el.innerHTML = `<p class="muted">No case loaded.</p>`;
  }
}

function row(k, v) {
  return `<tr><td style="width:180px;color:var(--fg-mute);text-transform:uppercase;font-size:10px;letter-spacing:.4px">${k}</td><td style="font-family:var(--mono)">${v}</td></tr>`;
}
