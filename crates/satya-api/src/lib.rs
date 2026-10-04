//! SATYA HTTP API + static frontend.

use axum::{
    body::Body,
    extract::{Path as AxumPath, Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::Mutex;
use tower_http::services::ServeDir;

mod image_fs;
mod case_api;
mod explorer_api;
mod juan_api;
mod juan_case;
mod report_api;
mod settings_api;

pub(crate) use case_api::log_event;

#[derive(Clone)]
pub struct AppState {
    inner: Arc<Mutex<StateInner>>,
    web_root: PathBuf,
}

#[derive(Default, Clone, Serialize)]
struct StateInner {
    image_path: String,
    video_path: String,
    oem: String,
    confidence: f32,
    sha256: String,
    md5: String,
    size_bytes: u64,
    allocated: usize,
    carved: usize,
    total_frames: usize,
    frames_json: Option<String>,
    timeline_json: Option<String>,
    summary_json: Option<String>,
    mp4_path: Option<PathBuf>,
    custody: Vec<serde_json::Value>,
    /// Set when the case is a JUAN/HeimVision image (streamed, E01 or raw).
    #[serde(skip)]
    juan_scan: Option<Arc<satya_parsers::juan::JuanScan>>,
}

pub fn router(state: AppState) -> Router {
    let web = ServeDir::new(&state.web_root);
    Router::new()
        .route("/api/health", get(|| async { "ok" }))
        .route("/api/case", get(get_case))
        .route("/api/identify", post(identify))
        .route("/api/analyze", post(analyze))
        .route("/api/frames", get(get_frames))
        .route("/api/timeline", get(get_timeline))
        .route("/api/custody", get(get_custody))
        .route("/api/export", post(export_video))
        .route("/api/video", get(stream_video))
        .route("/api/load-video", post(load_video))
        .route("/api/hash", post(hash_image))
        .route("/api/frame/:idx/hex", get(frame_hex))
        .route("/api/device", get(get_device))
        .route("/api/partitions", get(get_partitions))
        .route("/api/channels", get(get_channels))
        .route("/api/logs", get(get_logs))
        .route("/api/allocation", get(get_allocation))
        .route("/api/stats", get(get_stats))
        .route("/api/summary", get(get_summary))
        .route("/api/ml/detect", post(ml_detect))
        .route("/api/ml/faces", post(ml_faces))
        .route("/api/ml/motion", post(ml_motion))
        .route("/api/ml/thumbnail", post(ml_thumbnail))
        .route("/api/ml/health", get(ml_health))
        .route("/api/mcp/tools", get(mcp_tools))
        .route("/api/report", post(generate_report))
        .route("/api/fs/list", get(fs_list))
        .route("/api/fs/roots", get(fs_roots))
        .route("/api/image/list", get(image_list))
        .route("/api/image/hex", get(image_hex))
        .route("/api/hex-at", get(hex_at))
        .route("/api/image/node", get(image_node))
        .route("/api/ml/persons", post(ml_persons))
        .route("/api/juan/scan", post(juan_api::scan))
        .route("/api/juan/status", get(juan_api::status))
        .route("/api/juan/slots", get(juan_api::slots))
        .route("/api/juan/export", post(juan_api::export))
        // Menu bar
        .route("/api/case/save", post(case_api::save_case))
        .route("/api/case/close", post(case_api::close_case))
        .route("/api/case/export-csv", post(case_api::export_csv))
        .route("/api/verify/start", post(case_api::verify_start))
        .route("/api/verify/status", get(case_api::verify_status))
        .route("/api/outputs", get(case_api::outputs))
        // Explorer: inside the evidence image
        .route("/api/x/image", get(explorer_api::x_image))
        .route("/api/x/list", get(explorer_api::x_list))
        .route("/api/x/info", get(explorer_api::x_info))
        .route("/api/x/read", get(explorer_api::x_read))
        .route("/api/x/raw", get(explorer_api::x_raw))
        .route("/api/x/extract", post(explorer_api::x_extract))
        .route("/api/x/play", post(explorer_api::x_play))
        // Explorer: local disk
        .route("/api/local/roots", get(explorer_api::local_roots))
        .route("/api/local/list", get(explorer_api::local_list))
        .route("/api/local/read", get(explorer_api::local_read))
        .route("/api/local/raw", get(explorer_api::local_raw))
        // AI provider settings (MCP tab)
        .route("/api/settings", get(settings_api::get).post(settings_api::put))
        .route("/api/settings/test", post(settings_api::test))
        .fallback_service(web)
        // The desktop window (WebKit) caches aggressively; without this it can
        // keep showing an old UI after an update.
        .layer(axum::middleware::map_response(no_cache))
        .with_state(state)
}

async fn no_cache(mut res: Response) -> Response {
    res.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    res
}

// ---------------------------------------------------------------------------
// Case
// ---------------------------------------------------------------------------

async fn get_case(State(state): State<AppState>) -> impl IntoResponse {
    let inner = state.inner.lock().await;
    Json(inner.clone()).into_response()
}

#[derive(Deserialize)]
struct IdentifyRequest { image_path: String }

#[derive(Serialize)]
struct IdentifyResponse {
    oem: String, confidence: f32,
    block_size: Option<u32>, model: Option<String>, firmware: Option<String>,
}

async fn identify(Json(req): Json<IdentifyRequest>) -> impl IntoResponse {
    let path = req.image_path.clone();
    if let Ok(Some(fp)) = tokio::task::spawn_blocking(move || juan_api::detect(&path)).await {
        return (StatusCode::OK, Json(IdentifyResponse {
            oem: fp.oem.as_str().into(), confidence: fp.confidence,
            block_size: fp.block_size, model: fp.model, firmware: fp.firmware,
        })).into_response();
    }
    if let Err(msg) = juan_api::legacy_guard(&req.image_path) {
        return (StatusCode::BAD_REQUEST, msg).into_response();
    }
    match juan_case::read_all(&req.image_path) {
        Ok(data) => match satya_parsers::identify_device(&data) {
            Some(fp) => (StatusCode::OK, Json(IdentifyResponse {
                oem: fp.oem.as_str().into(), confidence: fp.confidence,
                block_size: fp.block_size, model: fp.model, firmware: fp.firmware,
            })).into_response(),
            None => (StatusCode::NOT_FOUND, "unknown OEM").into_response(),
        },
        Err(e) => (StatusCode::BAD_REQUEST, format!("read: {e}")).into_response(),
    }
}

#[derive(Deserialize)]
struct AnalyzeRequest { image_path: String }

#[derive(Serialize)]
struct AnalyzeResponse {
    oem: String, confidence: f32, frames: usize,
    allocated: usize, carved: usize,
    sha256: String, md5: String, size_bytes: u64,
    timeline_entries: usize,
}

async fn analyze(
    State(state): State<AppState>,
    Json(req): Json<AnalyzeRequest>,
) -> impl IntoResponse {
    let path = req.image_path.clone();
    if let Ok(Some(_)) = tokio::task::spawn_blocking(move || juan_api::detect(&path)).await {
        return analyze_juan(state, req.image_path).await;
    }
    if let Err(msg) = juan_api::legacy_guard(&req.image_path) {
        return (StatusCode::BAD_REQUEST, msg).into_response();
    }
    let data = match juan_case::read_all(&req.image_path) {
        Ok(d) => d,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };
    let fp = match satya_parsers::identify_device(&data) {
        Some(f) => f,
        None => return (StatusCode::NOT_FOUND, "unknown OEM").into_response(),
    };
    let frames = match satya_parsers::enumerate_frames(&data, fp.oem) {
        Ok(f) => f,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    };

    let allocated = frames.iter()
        .filter(|f| matches!(f.recovery_source, satya_core::RecoverySource::Allocated))
        .count();

    use sha2::{Digest, Sha256};
    use md5::Md5;
    let sha256 = hex::encode(Sha256::digest(&data));
    let md5 = hex::encode(Md5::digest(&data));

    use satya_trust::{fuse_with_diagnostics, to_claim, Source};
    let mut timeline = Vec::new();
    for frame in &frames {
        let mut claims = Vec::new();
        for c in &frame.claims {
            let src = match c.source {
                satya_core::TimestampSource::FrameHeader => Source::FrameHeader,
                satya_core::TimestampSource::DeviceLog => Source::DeviceLog,
                satya_core::TimestampSource::OnScreenOcr => Source::OnScreenOcr,
                satya_core::TimestampSource::NtpSyncLog => Source::NtpSyncLog,
                satya_core::TimestampSource::SeiMetadata => Source::FrameHeader,
            };
            claims.push(to_claim(c.claimed_utc.timestamp() as f64, src, c.confidence as f64));
        }
        if claims.is_empty() { continue; }
        timeline.push(fuse_with_diagnostics(&claims));
    }

    let frames_json = serde_json::to_string(&frames).ok();
    let timeline_json = serde_json::to_string(&timeline).ok();

    let mut inner = state.inner.lock().await;
    inner.image_path = req.image_path.clone();
    inner.oem = fp.oem.as_str().into();
    inner.confidence = fp.confidence;
    inner.sha256 = sha256.clone();
    inner.md5 = md5.clone();
    inner.size_bytes = data.len() as u64;
    inner.allocated = allocated;
    inner.carved = frames.len() - allocated;
    inner.total_frames = frames.len();
    inner.frames_json = frames_json;
    inner.timeline_json = timeline_json;
    inner.mp4_path = None;
    inner.juan_scan = None;

    let summary = satya_parsers::analysis::build_summary(&data, &fp, &frames);
    inner.summary_json = serde_json::to_string(&summary).ok();

    // Auto-export video so the Viewer tab works immediately.
    if let Some((start, end)) = satya_parsers::video_region::locate(&data, fp.oem) {
        let payload = &data[start..end];
        let h264 = PathBuf::from(&req.image_path).with_extension("h264");
        let mp4 = PathBuf::from(&req.image_path).with_extension("mp4");
        if payload.len() > 1024 {
            if std::fs::write(&h264, payload).is_ok() {
                if satya_video::h264_to_mp4(&h264, &mp4).is_ok() {
                    inner.mp4_path = Some(mp4.clone());
                    inner.video_path = mp4.to_string_lossy().to_string();
                    tracing::info!("auto-exported video → {}", mp4.display());
                }
            }
        }
    }

    drop(inner);
    log_event(&state, serde_json::json!({
        "action": "analyze", "image": req.image_path, "oem": fp.oem.as_str(),
        "frames": frames.len(), "sha256": sha256,
    })).await;

    Json(AnalyzeResponse {
        oem: fp.oem.as_str().into(),
        confidence: fp.confidence,
        frames: frames.len(),
        allocated,
        carved: frames.len() - allocated,
        sha256, md5,
        size_bytes: data.len() as u64,
        timeline_entries: timeline.len(),
    }).into_response()
}

/// Streaming analysis of a JUAN/HeimVision image (E01 or raw). Fills the
/// same case state as the legacy path, so every tab of the UI works.
async fn analyze_juan(state: AppState, image_path: String) -> Response {
    let path = image_path.clone();
    let case = match tokio::task::spawn_blocking(move || juan_case::analyze(&path)).await {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("worker: {e}")).into_response(),
    };
    let allocated = case.frames.iter()
        .filter(|f| matches!(f.recovery_source, satya_core::RecoverySource::Allocated))
        .count();

    let mut inner = state.inner.lock().await;
    inner.image_path = image_path;
    inner.oem = case.fp.oem.as_str().into();
    inner.confidence = case.fp.confidence;
    inner.sha256 = case.sha256.clone();
    inner.md5 = case.md5.clone();
    inner.size_bytes = case.size_bytes;
    inner.allocated = allocated;
    inner.carved = case.frames.len() - allocated;
    inner.total_frames = case.frames.len();
    inner.frames_json = serde_json::to_string(&case.frames).ok();
    inner.timeline_json = serde_json::to_string(&case.timeline).ok();
    inner.summary_json = serde_json::to_string(&case.summary).ok();
    inner.juan_scan = Some(case.scan.clone());
    inner.mp4_path = case.preview_mp4.clone();
    inner.video_path = case.preview_mp4.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    if let Some(p) = &case.preview_mp4 {
        tracing::info!("JUAN preview (H.264 viewing copy) -> {}", p.display());
    }
    drop(inner);
    log_event(&state, serde_json::json!({
        "action": "analyze", "image": inner_image_name(&state).await, "oem": case.fp.oem.as_str(),
        "slots": case.frames.len(), "stored_md5": case.md5,
    })).await;

    Json(AnalyzeResponse {
        oem: case.fp.oem.as_str().into(),
        confidence: case.fp.confidence,
        frames: case.frames.len(),
        allocated,
        carved: case.frames.len() - allocated,
        sha256: case.sha256,
        md5: case.md5,
        size_bytes: case.size_bytes,
        timeline_entries: case.timeline.len(),
    }).into_response()
}

async fn inner_image_name(state: &AppState) -> String {
    let p = state.inner.lock().await.image_path.clone();
    Path::new(&p).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or(p)
}

async fn get_frames(State(state): State<AppState>) -> impl IntoResponse {
    let inner = state.inner.lock().await;
    match &inner.frames_json {
        Some(j) => (StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")], j.clone()).into_response(),
        None => (StatusCode::NOT_FOUND, "no case loaded").into_response(),
    }
}

async fn get_timeline(State(state): State<AppState>) -> impl IntoResponse {
    let inner = state.inner.lock().await;
    match &inner.timeline_json {
        Some(j) => (StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")], j.clone()).into_response(),
        None => (StatusCode::NOT_FOUND, "no case loaded").into_response(),
    }
}

async fn get_custody(State(state): State<AppState>) -> impl IntoResponse {
    let inner = state.inner.lock().await;
    Json(&inner.custody).into_response()
}

// ---------------------------------------------------------------------------
// Video
// ---------------------------------------------------------------------------

async fn export_video(State(state): State<AppState>) -> impl IntoResponse {
    let (image_path, juan_scan) = {
        let inner = state.inner.lock().await;
        if inner.image_path.is_empty() {
            return (StatusCode::BAD_REQUEST, "analyze a case first").into_response();
        }
        (inner.image_path.clone(), inner.juan_scan.clone())
    };
    if let Some(scan) = juan_scan {
        // Full export of every recorded slot. The Viewer keeps playing the
        // H.264 preview; the exported MP4 is H.265 (play it in VLC/mpv).
        let path = image_path.clone();
        return match tokio::task::spawn_blocking(move || juan_case::export_all(&path, &scan)).await {
            Ok(Ok(r)) => {
                let mp4 = r.mp4_path.clone().unwrap_or_else(|| r.hevc_path.clone());
                let size = std::fs::metadata(&mp4).map(|m| m.len()).unwrap_or(0);
                log_event(&state, serde_json::json!({
                    "action": "export", "file": r.hevc_path, "bytes": r.report.bytes_written,
                    "md5": r.report.md5, "sha256": r.report.sha256, "manifest": r.manifest_path,
                })).await;
                Json(serde_json::json!({
                    "mp4": mp4,
                    "size_bytes": size,
                    "hevc": r.hevc_path,
                    "manifest": r.manifest_path,
                    "sha256": r.report.sha256,
                    "md5": r.report.md5,
                    "mp4_error": r.mp4_error,
                    "url": "/api/video",
                })).into_response()
            }
            Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("worker: {e}")).into_response(),
        };
    }
    let img = PathBuf::from(&image_path);
    let h264 = img.with_extension("h264");
    let mp4 = img.with_extension("mp4");

    if let Err(msg) = juan_api::legacy_guard(&image_path) {
        return (StatusCode::BAD_REQUEST, msg).into_response();
    }
    let data = match juan_case::read_all(&image_path) {
        Ok(d) => d,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let fp = match satya_parsers::identify_device(&data) {
        Some(f) => f,
        None => return (StatusCode::NOT_FOUND, "unknown OEM").into_response(),
    };

    // Locate the contiguous video payload — this is safer than carving
    // every NAL boundary because FFmpeg does the demuxing properly.
    let (start, end) = match satya_parsers::video_region::locate(&data, fp.oem) {
        Some(r) => r,
        None => return (StatusCode::INTERNAL_SERVER_ERROR, "cannot locate video region").into_response(),
    };
    let payload = &data[start..end];
    if payload.len() < 1024 {
        return (StatusCode::INTERNAL_SERVER_ERROR, "video region too small").into_response();
    }

    if let Err(e) = std::fs::write(&h264, payload) {
        return (StatusCode::INTERNAL_SERVER_ERROR, format!("write h264: {e}")).into_response();
    }

    match satya_video::h264_to_mp4(&h264, &mp4) {
        Ok(()) => {
            let size = std::fs::metadata(&mp4).map(|m| m.len()).unwrap_or(0);
            let mut inner = state.inner.lock().await;
            inner.mp4_path = Some(mp4.clone());
            inner.video_path = mp4.to_string_lossy().to_string();
            Json(serde_json::json!({
                "mp4": mp4.to_string_lossy(),
                "size_bytes": size,
                "video_region": [start, end],
                "url": "/api/video",
            })).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("ffmpeg: {e}")).into_response(),
    }
}

#[derive(Deserialize)]
struct LoadVideoRequest { path: String }

async fn load_video(
    State(state): State<AppState>,
    Json(req): Json<LoadVideoRequest>,
) -> impl IntoResponse {
    let p = PathBuf::from(&req.path);
    if !p.exists() {
        return (StatusCode::NOT_FOUND, "file not found").into_response();
    }
    let ext = p.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    if !matches!(ext.as_str(), "mp4" | "h264" | "mkv" | "webm" | "avi") {
        return (StatusCode::BAD_REQUEST, "unsupported video format").into_response();
    }
    let mut inner = state.inner.lock().await;
    inner.mp4_path = Some(p.clone());
    inner.video_path = p.to_string_lossy().to_string();
    Json(serde_json::json!({
        "loaded": p.to_string_lossy(),
        "url": "/api/video",
    })).into_response()
}

#[derive(Deserialize)]
struct VideoQuery { path: Option<String> }

/// Stream the current video with HTTP Range support so browsers can seek.
async fn stream_video(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Query(q): Query<VideoQuery>,
) -> impl IntoResponse {
    // Prefer explicit path, else current mp4_path
    let path = if let Some(p) = q.path {
        PathBuf::from(p)
    } else {
        let inner = state.inner.lock().await;
        match &inner.mp4_path {
            Some(p) => p.clone(),
            None => return (StatusCode::NOT_FOUND, "no video loaded").into_response(),
        }
    };

    let meta = match tokio::fs::metadata(&path).await {
        Ok(m) => m,
        Err(e) => return (StatusCode::NOT_FOUND, format!("stat: {e}")).into_response(),
    };
    let total = meta.len();
    let content_type = content_type_for(&path);

    // Parse Range
    let range = headers.get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| parse_range(s, total));

    if let Some((start, end)) = range {
        let length = end - start + 1;
        let data = match read_range(&path, start, length).await {
            Ok(d) => d,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("read: {e}")).into_response(),
        };
        let mut resp = Response::new(Body::from(data));
        resp.headers_mut().insert(header::CONTENT_TYPE,
            HeaderValue::from_str(&content_type).unwrap());
        resp.headers_mut().insert(header::ACCEPT_RANGES,
            HeaderValue::from_static("bytes"));
        resp.headers_mut().insert(header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{end}/{total}")).unwrap());
        resp.headers_mut().insert(header::CONTENT_LENGTH,
            HeaderValue::from_str(&length.to_string()).unwrap());
        *resp.status_mut() = StatusCode::PARTIAL_CONTENT;
        resp
    } else {
        let data = match tokio::fs::read(&path).await {
            Ok(d) => d,
            Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("read: {e}")).into_response(),
        };
        let mut resp = Response::new(Body::from(data));
        resp.headers_mut().insert(header::CONTENT_TYPE,
            HeaderValue::from_str(&content_type).unwrap());
        resp.headers_mut().insert(header::ACCEPT_RANGES,
            HeaderValue::from_static("bytes"));
        resp
    }
}

fn content_type_for(p: &Path) -> String {
    match p.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase().as_str() {
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "h264" => "video/h264",
        "avi" => "video/x-msvideo",
        _ => "application/octet-stream",
    }.to_string()
}

fn parse_range(header: &str, total: u64) -> Option<(u64, u64)> {
    let s = header.strip_prefix("bytes=")?;
    let (start_s, end_s) = s.split_once('-')?;
    let start: u64 = start_s.trim().parse().ok()?;
    let end: u64 = if end_s.trim().is_empty() {
        total.saturating_sub(1)
    } else {
        end_s.trim().parse().ok()?
    };
    if start > end || end >= total { return None; }
    Some((start, end))
}

async fn read_range(path: &Path, start: u64, length: u64) -> std::io::Result<Vec<u8>> {
    use tokio::io::{AsyncReadExt, AsyncSeekExt};
    let mut f = tokio::fs::File::open(path).await?;
    f.seek(std::io::SeekFrom::Start(start)).await?;
    let mut buf = vec![0u8; length as usize];
    f.read_exact(&mut buf).await?;
    Ok(buf)
}

async fn hash_image(Json(req): Json<IdentifyRequest>) -> impl IntoResponse {
    // Streams the logical media (for E01: the acquired disk, not the container).
    let path = req.image_path.clone();
    let res = tokio::task::spawn_blocking(move || -> Result<satya_image::hashing::MediaHashes, String> {
        let src = satya_image::open_image(Path::new(&path)).map_err(|e| format!("open: {e}"))?;
        satya_image::hashing::hash_media(src.as_ref(), |_, _| {}).map_err(|e| format!("read: {e}"))
    }).await;
    match res {
        Ok(Ok(h)) => Json(serde_json::json!({
            "sha256": h.sha256, "md5": h.md5, "sha1": h.sha1, "size_bytes": h.bytes,
            "stored_md5": h.stored_md5, "md5_matches_stored": h.md5_matches,
        })).into_response(),
        Ok(Err(e)) => (StatusCode::BAD_REQUEST, e).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("worker: {e}")).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Frame details
// ---------------------------------------------------------------------------

async fn frame_hex(
    State(state): State<AppState>,
    AxumPath(idx): AxumPath<usize>,
) -> impl IntoResponse {
    let (frames_json, image_path) = {
        let inner = state.inner.lock().await;
        (inner.frames_json.clone(), inner.image_path.clone())
    };
    let Some(fj) = frames_json else {
        return (StatusCode::NOT_FOUND, "no case").into_response();
    };
    let frames: Vec<serde_json::Value> = match serde_json::from_str(&fj) {
        Ok(v) => v, Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    };
    let Some(frame) = frames.get(idx) else {
        return (StatusCode::NOT_FOUND, "no such frame").into_response();
    };
    let offset = frame["offset"].as_u64().unwrap_or(0) as usize;
    let length = frame["length"].as_u64().unwrap_or(0) as usize;
    let data = match juan_case::read_range(&image_path, offset as u64, length.min(4096)) {
        Ok((_, d)) => d, Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let slice = &data[..];

    let mut out = String::new();
    for (i, chunk) in slice.chunks(16).enumerate() {
        out.push_str(&format!("{:08x}  ", offset + i * 16));
        for b in chunk { out.push_str(&format!("{:02x} ", b)); }
        for _ in chunk.len()..16 { out.push_str("   "); }
        out.push_str(" |");
        for b in chunk {
            let c = if (0x20..0x7f).contains(b) { *b as char } else { '.' };
            out.push(c);
        }
        out.push_str("|\n");
    }
    (StatusCode::OK, [(header::CONTENT_TYPE, "text/plain")], out).into_response()
}

// ---------------------------------------------------------------------------
// Analysis detail endpoints
// ---------------------------------------------------------------------------

async fn get_summary(State(state): State<AppState>) -> impl IntoResponse {
    let inner = state.inner.lock().await;
    match &inner.summary_json {
        Some(j) => (StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")], j.clone()).into_response(),
        None => (StatusCode::NOT_FOUND, "no case loaded").into_response(),
    }
}

async fn get_device(State(state): State<AppState>) -> impl IntoResponse { summary_field(state, "device").await }
async fn get_partitions(State(state): State<AppState>) -> impl IntoResponse { summary_field(state, "partitions").await }
async fn get_channels(State(state): State<AppState>) -> impl IntoResponse { summary_field(state, "channels").await }
async fn get_logs(State(state): State<AppState>) -> impl IntoResponse { summary_field(state, "logs").await }
async fn get_allocation(State(state): State<AppState>) -> impl IntoResponse { summary_field(state, "allocation").await }
async fn get_stats(State(state): State<AppState>) -> impl IntoResponse { summary_field(state, "stats").await }

async fn summary_field(state: AppState, key: &str) -> Response {
    let inner = state.inner.lock().await;
    let Some(j) = &inner.summary_json else {
        return (StatusCode::NOT_FOUND, "no case loaded").into_response();
    };
    let v: serde_json::Value = match serde_json::from_str(j) {
        Ok(v) => v, Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    };
    match v.get(key) {
        Some(field) => (StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            field.to_string()).into_response(),
        None => (StatusCode::NOT_FOUND, format!("missing field: {key}")).into_response(),
    }
}

// ---------------------------------------------------------------------------
// ML proxies
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct MLDetectRequest { frame_index: usize }

#[derive(Deserialize)]
struct MLMotionRequest {
    start_frame: usize, end_frame: usize,
    step: usize, threshold: f32,
}

#[derive(Deserialize)]
struct MLThumbRequest { frame_index: usize, width: usize }

async fn ml_health() -> impl IntoResponse {
    match call_ml_get("/health").await {
        Ok(b) => (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], b).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("ML sidecar: {e}")).into_response(),
    }
}

async fn ml_detect(
    State(state): State<AppState>,
    Json(req): Json<MLDetectRequest>,
) -> impl IntoResponse {
    let mp4 = { let i = state.inner.lock().await; i.mp4_path.clone() };
    let Some(mp4) = mp4 else {
        return (StatusCode::BAD_REQUEST, "export MP4 first (click Export MP4)").into_response();
    };
    let payload = serde_json::json!({
        "video_path": mp4.to_string_lossy(),
        "frame_index": req.frame_index,
    });
    match call_ml_post("/detect", &payload).await {
        Ok(b) => (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], b).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("ML sidecar: {e}")).into_response(),
    }
}

async fn ml_faces(
    State(state): State<AppState>,
    Json(req): Json<MLDetectRequest>,
) -> impl IntoResponse {
    let mp4 = { let i = state.inner.lock().await; i.mp4_path.clone() };
    let Some(mp4) = mp4 else {
        return (StatusCode::BAD_REQUEST, "export MP4 first").into_response();
    };
    let payload = serde_json::json!({
        "video_path": mp4.to_string_lossy(),
        "frame_index": req.frame_index,
    });
    match call_ml_post("/faces", &payload).await {
        Ok(b) => (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], b).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("ML sidecar: {e}")).into_response(),
    }
}

async fn ml_motion(
    State(state): State<AppState>,
    Json(req): Json<MLMotionRequest>,
) -> impl IntoResponse {
    let mp4 = { let i = state.inner.lock().await; i.mp4_path.clone() };
    let Some(mp4) = mp4 else {
        return (StatusCode::BAD_REQUEST, "export MP4 first").into_response();
    };
    let payload = serde_json::json!({
        "video_path": mp4.to_string_lossy(),
        "start_frame": req.start_frame,
        "end_frame": req.end_frame,
        "step": req.step.max(1),
        "threshold": req.threshold,
    });
    match call_ml_post("/motion", &payload).await {
        Ok(b) => (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], b).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("ML sidecar: {e}")).into_response(),
    }
}

async fn ml_thumbnail(
    State(state): State<AppState>,
    Json(req): Json<MLThumbRequest>,
) -> impl IntoResponse {
    let mp4 = { let i = state.inner.lock().await; i.mp4_path.clone() };
    let Some(mp4) = mp4 else {
        return (StatusCode::BAD_REQUEST, "export MP4 first").into_response();
    };
    let payload = serde_json::json!({
        "video_path": mp4.to_string_lossy(),
        "frame_index": req.frame_index,
        "width": req.width,
    });
    match call_ml_post("/thumbnail", &payload).await {
        Ok(b) => (StatusCode::OK, [(header::CONTENT_TYPE, "application/json")], b).into_response(),
        Err(e) => (StatusCode::BAD_GATEWAY, format!("ML sidecar: {e}")).into_response(),
    }
}

fn ml_base() -> String {
    std::env::var("SATYA_ML_URL").unwrap_or_else(|_| "http://127.0.0.1:8001".into())
}

async fn call_ml_get(path: &str) -> Result<String, String> {
    let url = format!("{}{}", ml_base(), path);
    let c = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build().map_err(|e| e.to_string())?;
    let r = c.get(&url).send().await.map_err(|e| format!("cannot reach {url}: {e}"))?;
    let status = r.status();
    let t = r.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() { return Err(format!("HTTP {status}: {t}")); }
    Ok(t)
}

async fn call_ml_post(path: &str, payload: &serde_json::Value) -> Result<String, String> {
    let url = format!("{}{}", ml_base(), path);
    let c = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build().map_err(|e| e.to_string())?;
    let r = c.post(&url).json(payload).send().await
        .map_err(|e| format!("cannot reach {url}: {e}"))?;
    let status = r.status();
    let t = r.text().await.map_err(|e| e.to_string())?;
    if !status.is_success() { return Err(format!("HTTP {status}: {t}")); }
    Ok(t)
}

async fn mcp_tools() -> impl IntoResponse {
    Json(serde_json::json!([
        {"name":"identify_oem","description":"Identify the OEM of a DVR disk image"},
        {"name":"enumerate_frames","description":"List all video frames including deleted ones"},
        {"name":"analyze_timeline","description":"Fuse timestamps into per-frame 95% confidence intervals"},
        {"name":"export_video","description":"Extract NAL units and wrap into playable MP4"},
        {"name":"hash_image","description":"Compute SHA-256 for evidence integrity"},
        {"name":"generate_report","description":"Produce a court-ready PDF report"},
        {"name":"verify_custody","description":"Verify a hybrid Ed25519 signature bundle"},
    ])).into_response()
}

async fn generate_report(
    State(state): State<AppState>,
    body: Option<Json<report_api::ReportOptions>>,
) -> impl IntoResponse {
    let (image_path, oem) = {
        let inner = state.inner.lock().await;
        (inner.image_path.clone(), inner.oem.clone())
    };
    if image_path.is_empty() {
        return (StatusCode::BAD_REQUEST, "no case").into_response();
    }

    // JUAN cases: detailed forensic report (evidence frames, timeline,
    // integrity, deleted entries, activity log), written to the output folder.
    let juan = {
        let i = state.inner.lock().await;
        i.juan_scan.clone().map(|scan| (scan, i.custody.clone()))
    };
    if let Some((scan, activity)) = juan {
        let opts = body.map(|Json(o)| o).unwrap_or_default();
        let img = image_path.clone();
        let res = tokio::task::spawn_blocking(move || report_api::build_and_write(&img, &scan, &opts, &activity)).await;
        return match res {
            Ok(Ok((pdf, sha256))) => {
                let size = std::fs::metadata(&pdf).map(|m| m.len()).unwrap_or(0);
                log_event(&state, serde_json::json!({"action": "report", "file": pdf, "bytes": size, "sha256": sha256})).await;
                Json(serde_json::json!({ "output_pdf": pdf, "size_bytes": size, "sha256": sha256 })).into_response()
            }
            Ok(Err(e)) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("worker: {e}")).into_response(),
        };
    }

    let out = PathBuf::from(&image_path).with_extension("report.pdf");
    if let Err(msg) = juan_api::legacy_guard(&image_path) {
        return (StatusCode::BAD_REQUEST, msg).into_response();
    }
    let data = match juan_case::read_all(&image_path) {
        Ok(d) => d, Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let fp = match satya_parsers::identify_device(&data) {
        Some(f) => f, None => return (StatusCode::NOT_FOUND, "unknown OEM").into_response(),
    };
    let frames = match satya_parsers::enumerate_frames(&data, fp.oem) {
        Ok(f) => f, Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    };

    use sha2::{Digest, Sha256};
    let sha256 = hex::encode(Sha256::digest(&data));
    let allocated = frames.iter()
        .filter(|f| matches!(f.recovery_source, satya_core::RecoverySource::Allocated))
        .count();

    use satya_trust::{fuse_with_diagnostics, to_claim, Source};
    let mut timeline = Vec::new();
    for frame in &frames {
        let mut claims = Vec::new();
        for c in &frame.claims {
            let src = match c.source {
                satya_core::TimestampSource::FrameHeader => Source::FrameHeader,
                satya_core::TimestampSource::DeviceLog => Source::DeviceLog,
                satya_core::TimestampSource::OnScreenOcr => Source::OnScreenOcr,
                satya_core::TimestampSource::NtpSyncLog => Source::NtpSyncLog,
                satya_core::TimestampSource::SeiMetadata => Source::FrameHeader,
            };
            claims.push(to_claim(c.claimed_utc.timestamp() as f64, src, c.confidence as f64));
        }
        if claims.is_empty() { continue; }
        timeline.push(fuse_with_diagnostics(&claims));
    }

    let summary_json = { let i = state.inner.lock().await; i.summary_json.clone() };
    let summary: serde_json::Value = summary_json
        .as_deref()
        .and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or(serde_json::json!({}));

    let input = satya_report::ReportInput {
        case_id: "auto".into(),
        case_name: format!("{} forensic analysis", oem.to_uppercase()),
        investigator: "SATYA".into(),
        organization: "SATYA Project".into(),
        evidence_path: image_path.clone(),
        oem: fp.oem.as_str().into(),
        sha256: sha256.clone(),
        md5: {
            use md5::Md5;
            hex::encode(Md5::digest(&data))
        },
        total_frames: frames.len(),
        allocated,
        carved: frames.len() - allocated,
        size_bytes: data.len() as u64,
        device: summary.get("device").cloned().unwrap_or(serde_json::json!({})),
        partitions: summary.get("partitions").cloned().unwrap_or(serde_json::json!([])),
        channels: summary.get("channels").cloned().unwrap_or(serde_json::json!([])),
        logs: summary.get("logs").cloned().unwrap_or(serde_json::json!([])),
        allocation: summary.get("allocation").cloned().unwrap_or(serde_json::json!([])),
        timeline,
        custody: Vec::new(),
    };

    match satya_report::generate_report(&input, &out) {
        Ok(()) => Json(serde_json::json!({
            "output_pdf": out.to_string_lossy(),
            "size_bytes": std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0),
        })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    }
}

// ---------------------------------------------------------------------------
// File manager
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct FsListQuery { path: Option<String> }

#[derive(Serialize)]
struct FsEntry {
    name: String, path: String,
    is_dir: bool, size_bytes: u64,
    modified: Option<String>,
    ext: String,
}

async fn fs_list(Query(q): Query<FsListQuery>) -> impl IntoResponse {
    let path = q.path.unwrap_or_else(|| {
        std::env::var("HOME").unwrap_or_else(|_| "/".into())
    });
    let p = PathBuf::from(&path);
    let rd = match std::fs::read_dir(&p) {
        Ok(r) => r, Err(e) => return (StatusCode::BAD_REQUEST, format!("read dir: {e}")).into_response(),
    };
    let mut entries: Vec<FsEntry> = Vec::new();
    for e in rd.flatten() {
        let meta = match e.metadata() { Ok(m) => m, Err(_) => continue };
        let name = e.file_name().to_string_lossy().to_string();
        if name.starts_with('.') { continue; }
        let full = e.path().to_string_lossy().to_string();
        let ext = e.path().extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
        let modified = meta.modified().ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_secs().to_string());
        entries.push(FsEntry {
            name, path: full, is_dir: meta.is_dir(),
            size_bytes: if meta.is_dir() { 0 } else { meta.len() },
            modified, ext,
        });
    }
    entries.sort_by(|a, b| {
        b.is_dir.cmp(&a.is_dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });

    Json(serde_json::json!({
        "path": p.to_string_lossy(),
        "parent": p.parent().map(|pp| pp.to_string_lossy().to_string()),
        "entries": entries,
    })).into_response()
}

async fn fs_roots() -> impl IntoResponse {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    Json(serde_json::json!({
        "home": home,
        "roots": ["/", home, format!("{home}/satya-images"), format!("{home}/Downloads")],
    })).into_response()
}


// ---------------------------------------------------------------------------
// Virtual filesystem over the DVR image
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct ImagePathQuery { path: Option<String>, max_bytes: Option<usize> }

async fn image_list(
    State(state): State<AppState>,
    Query(q): Query<ImagePathQuery>,
) -> impl IntoResponse {
    let (summary_json, frames_json, image_size) = {
        let inner = state.inner.lock().await;
        (
            inner.summary_json.clone(),
            inner.frames_json.clone(),
            inner.size_bytes,
        )
    };
    let Some(sj) = summary_json else {
        return (StatusCode::NOT_FOUND, "no case loaded — analyze first").into_response();
    };
    let summary: serde_json::Value = match serde_json::from_str(&sj) {
        Ok(v) => v, Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    };
    let frames: Vec<serde_json::Value> = frames_json.as_deref()
        .and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default();

    let fs = image_fs::ImageFs::new(&summary, &frames, image_size);
    let path = q.path.unwrap_or_else(|| "/".into());

    match fs.list(&path) {
        Ok(nodes) => Json(serde_json::json!({
            "path": path,
            "nodes": nodes,
        })).into_response(),
        Err(e) => (StatusCode::BAD_REQUEST, e).into_response(),
    }
}

async fn image_hex(
    State(state): State<AppState>,
    Query(q): Query<ImagePathQuery>,
) -> impl IntoResponse {
    let (summary_json, frames_json, image_path) = {
        let inner = state.inner.lock().await;
        (inner.summary_json.clone(), inner.frames_json.clone(), inner.image_path.clone())
    };
    let Some(sj) = summary_json else {
        return (StatusCode::NOT_FOUND, "no case loaded").into_response();
    };
    let Some(path) = q.path else {
        return (StatusCode::BAD_REQUEST, "?path= required").into_response();
    };
    let summary: serde_json::Value = match serde_json::from_str(&sj) {
        Ok(v) => v, Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    };
    let frames: Vec<serde_json::Value> = frames_json.as_deref()
        .and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default();

    let max_bytes = q.max_bytes.unwrap_or(4096).min(65536);
    let fs = image_fs::ImageFs::new(&summary, &frames, 0);

    let (start, end) = match fs.byte_range(&path, max_bytes) {
        Ok(r) => r,
        Err(e) => return (StatusCode::BAD_REQUEST, e).into_response(),
    };

    let data = match juan_case::read_range(&image_path, start, end.saturating_sub(start) as usize) {
        Ok((_, d)) => d, Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let slice = &data[..];

    let mut out = String::new();
    for (i, chunk) in slice.chunks(16).enumerate() {
        out.push_str(&format!("{:08x}  ", start as usize + i * 16));
        for b in chunk { out.push_str(&format!("{:02x} ", b)); }
        for _ in chunk.len()..16 { out.push_str("   "); }
        out.push_str(" |");
        for b in chunk {
            let c = if (0x20..0x7f).contains(b) { *b as char } else { '.' };
            out.push(c);
        }
        out.push_str("|\n");
    }

    // Also produce JSON details for the same node
    let detail_json = node_detail(&summary, &frames, &path);

    let payload = serde_json::json!({
        "path": path,
        "start": start,
        "end": end,
        "size": slice.len(),
        "hex": out,
        "detail": detail_json,
    });
    Json(payload).into_response()
}

async fn image_node(
    State(state): State<AppState>,
    Query(q): Query<ImagePathQuery>,
) -> impl IntoResponse {
    let (summary_json, frames_json) = {
        let inner = state.inner.lock().await;
        (inner.summary_json.clone(), inner.frames_json.clone())
    };
    let Some(sj) = summary_json else {
        return (StatusCode::NOT_FOUND, "no case loaded").into_response();
    };
    let Some(path) = q.path else {
        return (StatusCode::BAD_REQUEST, "?path= required").into_response();
    };
    let summary: serde_json::Value = serde_json::from_str(&sj).unwrap_or(serde_json::Value::Null);
    let frames: Vec<serde_json::Value> = frames_json.as_deref()
        .and_then(|j| serde_json::from_str(j).ok())
        .unwrap_or_default();
    match node_detail(&summary, &frames, &path) {
        Some(v) => Json(v).into_response(),
        None => (StatusCode::NOT_FOUND, "no detail for path").into_response(),
    }
}

fn node_detail(
    summary: &serde_json::Value,
    frames: &[serde_json::Value],
    path: &str,
) -> Option<serde_json::Value> {
    let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
    match parts.as_slice() {
        ["device"] => summary.get("device").cloned(),
        ["partitions", n] => summary["partitions"].get(n.parse::<usize>().ok()?).cloned(),
        ["logs", n] => summary["logs"].get(n.parse::<usize>().ok()?).cloned(),
        ["frames", group, n] => {
            let target = if *group == "allocated" { "Allocated" } else { "Carved" };
            let filtered: Vec<&serde_json::Value> = frames.iter()
                .filter(|f| f["recovery_source"].as_str() == Some(target))
                .collect();
            filtered.get(n.parse::<usize>().ok()?).map(|v| (*v).clone())
        }
        ["channels", n] => summary["channels"].get(n.parse::<usize>().ok()?).cloned(),
        ["unallocated", n] => {
            let target = n.parse::<usize>().ok()?;
            let mut count = 0;
            for r in summary["allocation"].as_array()? {
                if r["kind"].as_str() == Some("unallocated") {
                    if count == target { return Some(r.clone()); }
                    count += 1;
                }
            }
            None
        }
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Person detection (batch YOLO over sampled frames)
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct PersonsRequest {
    #[serde(default = "default_persons_sample")]
    sample_every: usize,
    #[serde(default = "default_persons_conf")]
    conf: f32,
}

fn default_persons_sample() -> usize { 25 }
fn default_persons_conf() -> f32 { 0.35 }

async fn ml_persons(
    State(state): State<AppState>,
    Json(req): Json<PersonsRequest>,
) -> impl IntoResponse {
    let mp4 = { let i = state.inner.lock().await; i.mp4_path.clone() };
    let Some(mp4) = mp4 else {
        return (StatusCode::BAD_REQUEST, "export MP4 first").into_response();
    };

    let payload = serde_json::json!({
        "video_path": mp4.to_string_lossy(),
        "sample_every": req.sample_every.max(1),
        "run_objects": true,
        "run_faces": false,
        "run_motion": false,
        "conf": req.conf,
    });

    let body = match call_ml_post("/analyze-all", &payload).await {
        Ok(b) => b,
        Err(e) => return (StatusCode::BAD_GATEWAY, format!("ML: {e}")).into_response(),
    };

    let v: serde_json::Value = match serde_json::from_str(&body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, format!("{e}")).into_response(),
    };

    let empty = vec![];
    let frames = v["frames"].as_array().unwrap_or(&empty);
    let mut persons = Vec::new();

    for f in frames {
        let fidx = f["frame_index"].as_u64().unwrap_or(0);
        let ts = f["timestamp_s"].as_f64();
        if let Some(objs) = f["objects"].as_array() {
            for o in objs {
                if o["label"].as_str() == Some("person") {
                    persons.push(serde_json::json!({
                        "frame_index": fidx,
                        "timestamp_s": ts,
                        "score": o["score"],
                        "bbox": o["bbox"],
                        "crop_png_b64": o.get("crop_png_b64").cloned()
                            .unwrap_or(serde_json::Value::Null),
                    }));
                }
            }
        }
    }

    Json(serde_json::json!({
        "video_path": mp4.to_string_lossy(),
        "sampled_frames": frames.len(),
        "sample_every": req.sample_every,
        "person_count": persons.len(),
        "persons": persons,
    })).into_response()
}


#[derive(Deserialize)]
struct HexAtQuery {
    offset: u64,
    length: Option<u64>,
}

async fn hex_at(
    State(state): State<AppState>,
    Query(q): Query<HexAtQuery>,
) -> impl IntoResponse {
    let image_path = {
        let inner = state.inner.lock().await;
        inner.image_path.clone()
    };
    if image_path.is_empty() {
        return (StatusCode::BAD_REQUEST, "no case loaded").into_response();
    }
    let length = q.length.unwrap_or(512).min(65536) as usize;
    let (offset, data) = match juan_case::read_range(&image_path, q.offset, length) {
        Ok((o, d)) => (o as usize, d), Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    };
    let slice = &data[..];

    let mut out = String::new();
    for (i, chunk) in slice.chunks(16).enumerate() {
        out.push_str(&format!("{:08x}  ", offset + i * 16));
        for b in chunk { out.push_str(&format!("{:02x} ", b)); }
        for _ in chunk.len()..16 { out.push_str("   "); }
        out.push_str(" |");
        for b in chunk {
            let c = if (0x20..0x7f).contains(b) { *b as char } else { '.' };
            out.push(c);
        }
        out.push_str("|\n");
    }

    Json(serde_json::json!({
        "offset": offset,
        "length": slice.len(),
        "hex": out,
    })).into_response()
}

// ---------------------------------------------------------------------------
// Entry
// ---------------------------------------------------------------------------

pub async fn run(addr: &str) -> anyhow::Result<()> {
    let web_root = locate_web_root();
    let state = AppState {
        inner: Arc::new(Mutex::new(StateInner::default())),
        web_root: web_root.clone(),
    };
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    tracing::info!("SATYA listening on http://{addr}");
    tracing::info!("serving frontend from {}", web_root.display());
    axum::serve(listener, app).await?;
    Ok(())
}

fn locate_web_root() -> PathBuf {
    if let Ok(p) = std::env::var("SATYA_WEB_ROOT") { return PathBuf::from(p); }
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors() {
            let c = ancestor.join("web");
            if c.join("index.html").exists() { return c; }
        }
    }
    PathBuf::from("web")
}
