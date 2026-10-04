async function post(path, body = {}) {
  const r = await fetch(path, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });
  if (!r.ok) throw new Error(await r.text());
  const ct = r.headers.get("content-type") || "";
  return ct.includes("application/json") ? r.json() : r.text();
}
async function get(path) {
  const r = await fetch(path);
  if (!r.ok) throw new Error(await r.text());
  const ct = r.headers.get("content-type") || "";
  return ct.includes("application/json") ? r.json() : r.text();
}
export const api = {
  case:        ()             => get("/api/case"),
  analyze:     (image_path)   => post("/api/analyze", { image_path }),
  frames:      ()             => get("/api/frames"),
  timeline:    ()             => get("/api/timeline"),
  custody:     ()             => get("/api/custody"),
  exportVideo: ()             => post("/api/export"),
  loadVideo:   (path)         => post("/api/load-video", { path }),
  hash:        (image_path)   => post("/api/hash", { image_path }),
  mcpTools:    ()             => get("/api/mcp/tools"),
  genReport:   (opts = {})    => post("/api/report", opts),
  frameHex:    (idx)          => get(`/api/frame/${idx}/hex`),
  videoUrl:    ()             => "/api/video",
  device:      ()             => get("/api/device"),
  channels:    ()             => get("/api/channels"),
  logs:        ()             => get("/api/logs"),
  allocation:  ()             => get("/api/allocation"),
  stats:       ()             => get("/api/stats"),
};
