//! JUAN (HeimVision/Zosi K9604-W family) endpoints.
//!
//! Unlike the legacy `/api/analyze`, nothing here loads the image into RAM:
//! the E01/raw image is read through `satya_image::ImageSource`. Scans and
//! exports run on a worker thread; the browser polls `/api/juan/status`.

use super::AppState;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    Json,
};
use chrono::{DateTime, Utc};
use satya_parsers::juan::{ExportOptions, ExportReport, JuanScan, JuanVolume, ScanOptions, SlotState};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

#[derive(Default, Clone, Serialize)]
pub struct JobView {
    pub status: String,
    pub image_path: String,
    pub done: usize,
    pub total: usize,
    pub started_utc: Option<DateTime<Utc>>,
    pub finished_utc: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub stored_md5: Option<String>,
    pub stored_sha1: Option<String>,
    pub media_bytes: u64,
    pub export: Option<ExportResult>,
}

#[derive(Clone, Serialize)]
pub struct ExportResult {
    pub hevc_path: String,
    pub manifest_path: String,
    pub mp4_path: Option<String>,
    pub mp4_error: Option<String>,
    pub fps_used: f64,
    pub report: ExportReport,
}

#[derive(Default)]
struct Job {
    view: JobView,
    scan: Option<Arc<JuanScan>>,
}

fn job() -> &'static Mutex<Job> {
    static JOB: OnceLock<Mutex<Job>> = OnceLock::new();
    JOB.get_or_init(|| Mutex::new(Job { view: JobView { status: "idle".into(), ..Default::default() }, scan: None }))
}

fn busy(j: &Job) -> bool {
    matches!(j.view.status.as_str(), "scanning" | "exporting")
}

fn fail(msg: String) {
    let mut j = job().lock().unwrap();
    j.view.status = "error".into();
    j.view.error = Some(msg);
    j.view.finished_utc = Some(Utc::now());
}

// ---------------------------------------------------------------------------
// POST /api/juan/scan
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ScanRequest {
    pub image_path: String,
    #[serde(default)]
    pub deep: bool,
    #[serde(default)]
    pub hash: bool,
}

pub async fn scan(Json(req): Json<ScanRequest>) -> impl IntoResponse {
    {
        let mut j = job().lock().unwrap();
        if busy(&j) {
            return (StatusCode::CONFLICT, format!("busy: {}", j.view.status)).into_response();
        }
        j.scan = None;
        j.view = JobView {
            status: "scanning".into(),
            image_path: req.image_path.clone(),
            started_utc: Some(Utc::now()),
            ..Default::default()
        };
    }
    std::thread::spawn(move || run_scan(req));
    (StatusCode::ACCEPTED, Json(job().lock().unwrap().view.clone())).into_response()
}

fn run_scan(req: ScanRequest) {
    let src = match satya_image::open_image(Path::new(&req.image_path)) {
        Ok(s) => s,
        Err(e) => return fail(format!("open image: {e}")),
    };
    {
        let h = src.stored_hashes();
        let mut j = job().lock().unwrap();
        j.view.stored_md5 = h.md5;
        j.view.stored_sha1 = h.sha1;
        j.view.media_bytes = src.len();
    }
    let vol = match JuanVolume::open(src.as_ref()) {
        Ok(Some(v)) => v,
        Ok(None) => return fail("no JUAN layout found (needs FAT32 with ident.bin, index.bin, dirNNNNN/fileNNNN.dat)".into()),
        Err(e) => return fail(format!("open volume: {e}")),
    };
    let opts = ScanOptions { deep: req.deep, hash_payloads: req.hash };
    let result = vol.scan(&opts, |done, total| {
        let mut j = job().lock().unwrap();
        j.view.done = done;
        j.view.total = total;
    });
    let mut j = job().lock().unwrap();
    match result {
        Ok(scan) => {
            j.scan = Some(Arc::new(scan));
            j.view.status = "done".into();
        }
        Err(e) => {
            j.view.status = "error".into();
            j.view.error = Some(format!("scan: {e}"));
        }
    }
    j.view.finished_utc = Some(Utc::now());
}

// ---------------------------------------------------------------------------
// GET /api/juan/status
// ---------------------------------------------------------------------------

#[derive(Serialize)]
struct StatusResponse {
    job: JobView,
    layout: Option<serde_json::Value>,
    summary: Option<serde_json::Value>,
    warnings: Vec<String>,
}

pub async fn status() -> impl IntoResponse {
    let j = job().lock().unwrap();
    let (layout, summary, warnings) = match &j.scan {
        Some(s) => (
            serde_json::to_value(&s.layout).ok(),
            serde_json::to_value(&s.summary).ok(),
            warnings_for(s),
        ),
        None => (None, None, Vec::new()),
    };
    Json(StatusResponse { job: j.view.clone(), layout, summary, warnings })
}

/// Plain-language flags for things an examiner must not assume.
pub fn warnings_for(s: &JuanScan) -> Vec<String> {
    let m = &s.summary;
    let mut w = Vec::new();
    if m.distinct_sps > 1 {
        w.push(format!(
            "{} distinct H.265 SPS found: slots likely interleave several cameras or resolutions. \
             A combined export mixes them; do not present it as one camera.",
            m.distinct_sps
        ));
    }
    if !m.slot_order_monotonic {
        w.push("Header start times go backwards in slot order: ring buffer has wrapped or the clock was changed.".into());
    }
    if m.overlaps > 0 {
        w.push(format!("{} slots start before the previous slot ends (parallel streams or clock change).", m.overlaps));
    }
    if m.residual > 0 {
        w.push(format!("{} header-less slots still hold data: recovery candidates (export with 'include residual').", m.residual));
    }
    if m.unrecognized > 0 {
        w.push(format!("{} slots hold data that is not a 'luo ' header: unknown structure, inspect manually.", m.unrecognized));
    }
    if m.header_len_hist.len() > 1 {
        w.push(format!("Header length varies ({} distinct values): do not hard-code it.", m.header_len_hist.len()));
    }
    if m.nal_nonstandard > 0 {
        w.push(format!(
            "{} non-standard units inside the H.265 stream (proprietary metadata or audio). Use 'strip' for clean playback.",
            m.nal_nonstandard
        ));
    }
    w
}

// ---------------------------------------------------------------------------
// GET /api/juan/slots?state=recorded&offset=0&limit=200
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct SlotsQuery {
    pub state: Option<String>,
    #[serde(default)]
    pub offset: usize,
    pub limit: Option<usize>,
}

pub async fn slots(Query(q): Query<SlotsQuery>) -> impl IntoResponse {
    let scan = match job().lock().unwrap().scan.clone() {
        Some(s) => s,
        None => return (StatusCode::NOT_FOUND, "no scan yet").into_response(),
    };
    let want = q.state.as_deref().unwrap_or("all").to_ascii_lowercase();
    let keep = |st: SlotState| match want.as_str() {
        "recorded" => st == SlotState::Recorded,
        "empty" => st == SlotState::Empty,
        "residual" => st == SlotState::Residual,
        "unrecognized" => st == SlotState::Unrecognized,
        "data" => st != SlotState::Empty,
        _ => true,
    };
    let filtered: Vec<_> = scan.slots.iter().filter(|r| keep(r.state)).collect();
    let limit = q.limit.unwrap_or(200).min(2000);
    let page: Vec<_> = filtered.iter().skip(q.offset).take(limit).collect();
    Json(serde_json::json!({ "total": filtered.len(), "offset": q.offset, "slots": page })).into_response()
}

// ---------------------------------------------------------------------------
// POST /api/juan/export
// ---------------------------------------------------------------------------

#[derive(Deserialize, Default)]
pub struct ExportRequest {
    pub from: Option<u32>,
    pub to: Option<u32>,
    #[serde(default)]
    pub strip: bool,
    #[serde(default)]
    pub residual: bool,
    /// Playback rate for the MP4 wrapper; defaults to the scan's estimate.
    pub fps: Option<f64>,
}

pub async fn export(State(state): State<AppState>, Json(req): Json<ExportRequest>) -> impl IntoResponse {
    let (scan, image_path) = {
        let mut j = job().lock().unwrap();
        if busy(&j) {
            return (StatusCode::CONFLICT, format!("busy: {}", j.view.status)).into_response();
        }
        let Some(scan) = j.scan.clone() else {
            return (StatusCode::BAD_REQUEST, "run a scan first").into_response();
        };
        j.view.status = "exporting".into();
        j.view.error = None;
        j.view.export = None;
        (scan, j.view.image_path.clone())
    };
    let res = tokio::task::spawn_blocking(move || run_export(&image_path, &scan, &req)).await;
    let res = match res {
        Ok(r) => r,
        Err(e) => Err(e.to_string()),
    };
    {
        let mut j = job().lock().unwrap();
        j.view.finished_utc = Some(Utc::now());
        match &res {
            Ok(out) => {
                j.view.status = "done".into();
                j.view.export = Some(out.clone());
            }
            Err(e) => {
                j.view.status = "error".into();
                j.view.error = Some(e.clone());
            }
        }
    }
    match res {
        Ok(out) => {
            if let Some(mp4) = &out.mp4_path {
                let mut inner = state.inner.lock().await;
                inner.mp4_path = Some(PathBuf::from(mp4));
                inner.video_path = mp4.clone();
            }
            Json(out).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// Output folder for exports and reports: $SATYA_OUT, else ./satya_out
/// (relative to where the app was started), always returned as an
/// absolute path so the UI shows exactly where files went.
pub fn out_dir() -> PathBuf {
    let d = std::env::var("SATYA_OUT").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from("satya_out"));
    if d.is_absolute() {
        d
    } else {
        std::env::current_dir().map(|c| c.join(&d)).unwrap_or(d)
    }
}

pub(crate) fn run_export(image_path: &str, scan: &JuanScan, req: &ExportRequest) -> Result<ExportResult, String> {
    let src = satya_image::open_image(Path::new(image_path)).map_err(|e| format!("open image: {e}"))?;
    let vol = JuanVolume::open(src.as_ref())
        .map_err(|e| format!("open volume: {e}"))?
        .ok_or("JUAN layout no longer detected")?;

    let dir = out_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let range = match (req.from, req.to) {
        (None, None) => "all".to_string(),
        (a, b) => format!("{}-{}", a.unwrap_or(0), b.map(|v| v.to_string()).unwrap_or_else(|| "end".into())),
    };
    let stem = format!("juan_slots_{range}{}", if req.strip { "_clean" } else { "" });
    let hevc = dir.join(format!("{stem}.hevc"));
    let manifest = dir.join(format!("{stem}.manifest.json"));

    let opts = ExportOptions {
        strip_nonstandard: req.strip,
        slot_range: (req.from.is_some() || req.to.is_some()).then(|| (req.from.unwrap_or(0), req.to.unwrap_or(u32::MAX))),
        include_residual: req.residual,
    };
    let started = Utc::now();
    let mut f = std::io::BufWriter::new(std::fs::File::create(&hevc).map_err(|e| format!("create {}: {e}", hevc.display()))?);
    let report = vol.export_hevc(scan, &mut f, &opts).map_err(|e| format!("export: {e}"))?;
    drop(f);

    let fps = req.fps.or(report.est_fps).filter(|v| v.is_finite() && *v >= 1.0 && *v <= 120.0).unwrap_or(25.0);
    let mp4 = hevc.with_extension("mp4");
    let (mp4_path, mp4_error) = match satya_video::hevc_to_mp4(&hevc, &mp4, fps) {
        Ok(()) => (Some(mp4.to_string_lossy().into_owned()), None),
        Err(e) => (None, Some(e.to_string())),
    };

    let written: std::collections::BTreeSet<u32> = report.slot_order.iter().copied().collect();
    let provenance: Vec<_> = scan
        .slots
        .iter()
        .filter(|r| written.contains(&r.slot))
        .map(|r| {
            serde_json::json!({
                "slot": r.slot, "path": r.path, "state": r.state,
                "header_start_utc": r.header_start_utc, "header_end_utc": r.header_end_utc,
                "fat_created_local": r.fat_created, "payload_offset_in_slot": r.payload_offset,
                "payload_len": r.payload_len, "disk_extents": r.payload_extents,
                "payload_sha256": r.payload_sha256,
            })
        })
        .collect();
    let stored = src.stored_hashes();
    let doc = serde_json::json!({
        "tool": format!("satya-api {}", env!("CARGO_PKG_VERSION")),
        "export_started_utc": started,
        "export_finished_utc": Utc::now(),
        "image": { "path": image_path, "kind": src.kind(), "media_bytes": src.len(),
                   "stored_md5": stored.md5, "stored_sha1": stored.sha1 },
        "layout": scan.layout,
        "options": { "from": req.from, "to": req.to, "strip_nonstandard": req.strip, "include_residual": req.residual },
        "output": { "hevc": hevc, "md5": report.md5, "sha256": report.sha256, "bytes": report.bytes_written,
                    "mp4": mp4_path, "mp4_fps": fps,
                    "mp4_note": "MP4 frame timing is a playback estimate, not evidence; use header times." },
        "warnings": warnings_for(scan),
        "slots": provenance,
    });
    std::fs::write(&manifest, serde_json::to_vec_pretty(&doc).unwrap_or_default())
        .map_err(|e| format!("write manifest: {e}"))?;

    Ok(ExportResult {
        hevc_path: hevc.to_string_lossy().into_owned(),
        manifest_path: manifest.to_string_lossy().into_owned(),
        mp4_path,
        mp4_error,
        fps_used: fps,
        report,
    })
}

/// Cheap streaming check used by the legacy endpoints before they read a
/// whole image into memory.
pub fn detect(path: &str) -> Option<satya_core::DeviceFingerprint> {
    let src = satya_image::open_image(Path::new(path)).ok()?;
    let vol = JuanVolume::open(src.as_ref()).ok()??;
    Some(satya_parsers::juan::fingerprint(&vol.layout))
}

/// Legacy analyzers hold the whole media in RAM (raw or E01, decompressed).
pub fn legacy_guard(path: &str) -> Result<(), String> {
    const LIMIT: u64 = 2 << 30;
    let src = satya_image::open_image(Path::new(path)).map_err(|e| format!("open: {e}"))?;
    if src.len() > LIMIT {
        return Err(format!(
            "image is {:.1} GB; the legacy analyzer loads it into RAM (limit 2 GB). Use the CLI for large images.",
            src.len() as f64 / 1e9
        ));
    }
    Ok(())
}
