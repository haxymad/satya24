//! File explorers.
//!
//! `/api/x/*`     files INSIDE the evidence image (FAT32 / ext partitions),
//!                read-only, including deleted entries.
//! `/api/local/*` files on this computer (to find and open evidence).
//!
//! Opened images and volumes are cached so browsing a 150 GB E01 stays fast
//! (the FAT alone is ~35 MB and is loaded once).

use super::{log_event, AppState};
use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use satya_image::vfs::{volumes, Volume};
use satya_image::{open_image, ImageSource, Partition};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

const MAX_RAW: u64 = 256 << 20;
const MAX_READ: usize = 64 << 10;

// ---------------------------------------------------------------------------
// Image handle cache
// ---------------------------------------------------------------------------

struct ImageHandle {
    src: &'static dyn ImageSource,
    parts: Vec<Partition>,
    vols: Mutex<HashMap<u32, Arc<Volume<'static>>>>,
}

fn cache() -> &'static Mutex<HashMap<String, Arc<ImageHandle>>> {
    static C: OnceLock<Mutex<HashMap<String, Arc<ImageHandle>>>> = OnceLock::new();
    C.get_or_init(Default::default)
}

fn handle(path: &str) -> Result<Arc<ImageHandle>, String> {
    if let Some(h) = cache().lock().unwrap().get(path) {
        return Ok(h.clone());
    }
    let src = open_image(Path::new(path)).map_err(|e| format!("open image: {e}"))?;
    // Kept for the life of the process (one reader per distinct image path),
    // so cached volumes can borrow it.
    let src: &'static dyn ImageSource = Box::leak(src);
    let h = Arc::new(ImageHandle { src, parts: volumes(src), vols: Mutex::new(HashMap::new()) });
    cache().lock().unwrap().insert(path.to_string(), h.clone());
    Ok(h)
}

fn volume(h: &ImageHandle, part: u32) -> Result<Arc<Volume<'static>>, String> {
    if let Some(v) = h.vols.lock().unwrap().get(&part) {
        return Ok(v.clone());
    }
    let p = h.parts.iter().find(|p| p.index == part).ok_or(format!("no partition {part}"))?;
    let v = Arc::new(Volume::open(h.src, p).map_err(|e| e.to_string())?);
    h.vols.lock().unwrap().insert(part, v.clone());
    Ok(v)
}

async fn image_path(state: &AppState, q: &Option<String>) -> Result<String, String> {
    match q {
        Some(p) if !p.is_empty() => Ok(p.clone()),
        _ => {
            let p = state.inner.lock().await.image_path.clone();
            if p.is_empty() { Err("no image: analyze one first or pass ?image=".into()) } else { Ok(p) }
        }
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<T, String> {
    tokio::task::spawn_blocking(f).await.map_err(|e| format!("worker: {e}"))?
}

fn err(code: StatusCode, msg: String) -> Response {
    (code, msg).into_response()
}

// ---------------------------------------------------------------------------
// Inside the image
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct ImgQ {
    image: Option<String>,
}

pub async fn x_image(State(state): State<AppState>, Query(q): Query<ImgQ>) -> Response {
    let path = match image_path(&state, &q.image).await { Ok(p) => p, Err(e) => return err(StatusCode::BAD_REQUEST, e) };
    let res = blocking(move || {
        let h = handle(&path)?;
        let parts: Vec<_> = h
            .parts
            .iter()
            .map(|p| {
                let vol = volume(&h, p.index);
                serde_json::json!({
                    "index": p.index, "scheme": p.scheme, "name": p.name, "type_id": p.type_id,
                    "start_lba": p.start_lba, "end_lba": p.end_lba, "bytes": p.size(), "offset": p.offset(),
                    "fs": p.fs.as_str(),
                    "browsable": vol.is_ok(),
                    "volume": vol.as_ref().ok().map(|v| v.info()),
                    "error": vol.err(),
                })
            })
            .collect();
        Ok(serde_json::json!({
            "image": path,
            "kind": h.src.kind(),
            "media_bytes": h.src.len(),
            "sectors": h.src.len() / 512,
            "stored": h.src.stored_hashes(),
            "acquisition": h.src.acquisition(),
            "partitions": parts,
        }))
    })
    .await;
    match res {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

#[derive(Deserialize)]
pub struct NodeQ {
    image: Option<String>,
    part: u32,
    #[serde(default = "root")]
    node: String,
    #[serde(default)]
    offset: u64,
    len: Option<usize>,
    #[serde(default)]
    mode: String,
    name: Option<String>,
}
fn root() -> String {
    "root".into()
}

pub async fn x_list(State(state): State<AppState>, Query(q): Query<NodeQ>) -> Response {
    let path = match image_path(&state, &q.image).await { Ok(p) => p, Err(e) => return err(StatusCode::BAD_REQUEST, e) };
    let res = blocking(move || {
        let h = handle(&path)?;
        let v = volume(&h, q.part)?;
        v.list(&q.node).map_err(|e| e.to_string())
    })
    .await;
    match res {
        Ok(entries) => Json(serde_json::json!({ "entries": entries })).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

pub async fn x_info(State(state): State<AppState>, Query(q): Query<NodeQ>) -> Response {
    let path = match image_path(&state, &q.image).await { Ok(p) => p, Err(e) => return err(StatusCode::BAD_REQUEST, e) };
    let res = blocking(move || {
        let h = handle(&path)?;
        let v = volume(&h, q.part)?;
        let (size, ext) = v.extents(&q.node).map_err(|e| e.to_string())?;
        let head = v.read(&q.node, 0, 4096).map_err(|e| e.to_string())?;
        let kind = sniff(&head, q.name.as_deref().unwrap_or(""));
        let slot = satya_parsers::juan::slot_header(&head).map(|(a, b)| {
            serde_json::json!({
                "start_unix": a, "end_unix": b,
                "start_utc": chrono::DateTime::from_timestamp(a as i64, 0),
                "end_utc": chrono::DateTime::from_timestamp(b as i64, 0),
                "seconds": b as i64 - a as i64,
            })
        });
        Ok(serde_json::json!({
            "size": size,
            "kind": kind.0, "mime": kind.1,
            "extent_count": ext.len(),
            "extents": ext.iter().take(32).collect::<Vec<_>>(),
            "first_sector": ext.first().map(|x| x.disk_offset / 512),
            "contiguous": ext.len() <= 1,
            "juan_slot": slot,
            "playable": slot.is_some() || kind.0 == "video stream",
        }))
    })
    .await;
    match res {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

pub async fn x_read(State(state): State<AppState>, Query(q): Query<NodeQ>) -> Response {
    let path = match image_path(&state, &q.image).await { Ok(p) => p, Err(e) => return err(StatusCode::BAD_REQUEST, e) };
    let res = blocking(move || {
        let h = handle(&path)?;
        let v = volume(&h, q.part)?;
        if q.mode == "strings" {
            let data = v.read(&q.node, q.offset, 4 << 20).map_err(|e| e.to_string())?;
            return Ok(serde_json::json!({ "offset": q.offset, "strings": strings(&data, q.offset, 2000) }));
        }
        let len = q.len.unwrap_or(4096).min(MAX_READ);
        let data = v.read(&q.node, q.offset, len).map_err(|e| e.to_string())?;
        Ok(dump_json(q.offset, &data))
    })
    .await;
    match res {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

/// Whole file bytes (for picture/PDF previews and downloads).
pub async fn x_raw(State(state): State<AppState>, Query(q): Query<NodeQ>) -> Response {
    let path = match image_path(&state, &q.image).await { Ok(p) => p, Err(e) => return err(StatusCode::BAD_REQUEST, e) };
    let name = q.name.clone().unwrap_or_else(|| "file.bin".into());
    let res = blocking(move || {
        let h = handle(&path)?;
        let v = volume(&h, q.part)?;
        let (size, _) = v.extents(&q.node).map_err(|e| e.to_string())?;
        if size > MAX_RAW {
            return Err(format!("file is {size} bytes; use Extract for files over {MAX_RAW} bytes"));
        }
        v.read(&q.node, 0, size as usize).map_err(|e| e.to_string())
    })
    .await;
    match res {
        Ok(data) => bytes_response(data, &name),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

#[derive(Deserialize)]
pub struct ExtractReq {
    image: Option<String>,
    part: u32,
    node: String,
    name: String,
    /// Display path inside the volume, used to mirror folders in the output.
    path: Option<String>,
}

/// Copy a file out of the image into the output folder, with hashes and a log line.
pub async fn x_extract(State(state): State<AppState>, Json(r): Json<ExtractReq>) -> Response {
    let path = match image_path(&state, &r.image).await { Ok(p) => p, Err(e) => return err(StatusCode::BAD_REQUEST, e) };
    let img = path.clone();
    let res = blocking(move || {
        let h = handle(&img)?;
        let v = volume(&h, r.part)?;
        let (size, ext) = v.extents(&r.node).map_err(|e| e.to_string())?;
        let rel = safe_rel(r.path.as_deref().unwrap_or(&r.name));
        let out = crate::juan_api::out_dir().join("extracted").join(format!("p{}", r.part)).join(rel);
        if let Some(d) = out.parent() {
            std::fs::create_dir_all(d).map_err(|e| format!("create {}: {e}", d.display()))?;
        }
        let mut f = std::io::BufWriter::new(std::fs::File::create(&out).map_err(|e| format!("create {}: {e}", out.display()))?);
        let (mut md5, mut sha) = (md5::Md5::new(), Sha256::new());
        let mut off = 0u64;
        while off < size {
            let chunk = v.read(&r.node, off, (4 << 20).min((size - off) as usize)).map_err(|e| e.to_string())?;
            if chunk.is_empty() {
                break;
            }
            use std::io::Write;
            f.write_all(&chunk).map_err(|e| e.to_string())?;
            md5.update(&chunk);
            sha.update(&chunk);
            off += chunk.len() as u64;
        }
        drop(f);
        let rec = serde_json::json!({
            "extracted_utc": chrono::Utc::now(),
            "image": img, "partition": r.part, "node": r.node, "name": r.name, "inside_path": r.path,
            "bytes": off, "md5": hex::encode(md5.finalize()), "sha256": hex::encode(sha.finalize()),
            "first_sector": ext.first().map(|x| x.disk_offset / 512), "extents": ext.len(),
            "output": out,
        });
        let log = crate::juan_api::out_dir().join("extracted").join("extraction_log.jsonl");
        use std::io::Write;
        if let Ok(mut l) = std::fs::OpenOptions::new().create(true).append(true).open(&log) {
            let _ = writeln!(l, "{rec}");
        }
        Ok(rec)
    })
    .await;
    match res {
        Ok(rec) => {
            log_event(&state, serde_json::json!({"action": "extract", "file": rec["inside_path"], "sha256": rec["sha256"], "output": rec["output"]})).await;
            Json(rec).into_response()
        }
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

#[derive(Deserialize)]
pub struct PlayReq {
    image: Option<String>,
    part: u32,
    node: String,
    name: String,
}

/// Make a browser-playable H.264 viewing copy of one recording slot.
pub async fn x_play(State(state): State<AppState>, Json(r): Json<PlayReq>) -> Response {
    let path = match image_path(&state, &r.image).await { Ok(p) => p, Err(e) => return err(StatusCode::BAD_REQUEST, e) };
    let res = blocking(move || {
        let h = handle(&path)?;
        let v = volume(&h, r.part)?;
        let (size, _) = v.extents(&r.node).map_err(|e| e.to_string())?;
        let data = v.read(&r.node, 0, size.min(MAX_RAW) as usize).map_err(|e| e.to_string())?;
        let (a, b) = satya_parsers::juan::slot_payload(&data).ok_or("no H.265 video found in this file")?;
        let stream = satya_parsers::juan::clean_hevc(&data[a..b]);
        let dir = crate::juan_api::out_dir().join("slots");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let stem = format!("p{}_{}", r.part, safe_rel(&r.name).to_string_lossy().replace(['/', '.'], "_"));
        let hevc = dir.join(format!("{stem}.hevc"));
        let mp4 = dir.join(format!("{stem}.mp4"));
        std::fs::write(&hevc, &stream).map_err(|e| e.to_string())?;
        let secs = satya_parsers::juan::slot_header(&data).map(|(s, e)| e.saturating_sub(s) as f64).unwrap_or(0.0);
        let pics = count_pictures(&stream);
        let fps = if secs > 0.0 { (pics as f64 / secs).clamp(1.0, 60.0) } else { 25.0 };
        satya_video::hevc_to_h264_preview(&hevc, &mp4, fps)
            .or_else(|_| satya_video::hevc_to_mp4(&hevc, &mp4, fps))
            .map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&hevc);
        Ok(serde_json::json!({ "mp4": mp4, "url": format!("/api/video?path={}", urlenc(&mp4.to_string_lossy())), "fps": fps }))
    })
    .await;
    match res {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

/// All deleted entries in browsable volumes (directories walked to `max_depth`).
pub(crate) fn deleted_entries(path: &str, max_depth: usize) -> Vec<serde_json::Value> {
    let Ok(h) = handle(path) else { return Vec::new() };
    let mut out = Vec::new();
    for p in &h.parts {
        let Ok(v) = volume(&h, p.index) else { continue };
        let mut stack = vec![("root".to_string(), String::new(), 0usize)];
        while let Some((node, prefix, depth)) = stack.pop() {
            let Ok(list) = v.list(&node) else { continue };
            for e in list {
                let full = format!("{prefix}/{}", e.name);
                if e.deleted {
                    out.push(serde_json::json!({
                        "partition": p.index, "fs": p.fs.as_str(), "path": full, "id": e.id,
                        "size": e.size, "deleted_at": e.deleted_at, "recoverable": e.recoverable, "note": e.note,
                    }));
                } else if e.is_dir && depth < max_depth {
                    stack.push((e.node.clone(), full, depth + 1));
                }
                if out.len() >= 500 {
                    return out;
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Local disk
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct LocalQ {
    path: Option<String>,
    #[serde(default)]
    hidden: bool,
    #[serde(default)]
    offset: u64,
    len: Option<usize>,
    #[serde(default)]
    mode: String,
}

pub async fn local_roots() -> Response {
    let home = std::env::var("HOME").unwrap_or_else(|_| "/".into());
    let user = std::env::var("USER").unwrap_or_default();
    let mut places = vec![("Home".to_string(), home.clone()), ("Computer".to_string(), "/".to_string())];
    for d in ["Downloads", "Desktop", "Documents", "Videos"] {
        let p = format!("{home}/{d}");
        if Path::new(&p).is_dir() {
            places.push((d.to_string(), p));
        }
    }
    let out = crate::juan_api::out_dir();
    if out.is_dir() {
        places.push(("SATYA output".into(), out.to_string_lossy().into_owned()));
    }
    let mut drives = Vec::new();
    for base in [format!("/media/{user}"), format!("/run/media/{user}"), "/mnt".to_string()] {
        if let Ok(rd) = std::fs::read_dir(&base) {
            for e in rd.flatten() {
                if e.path().is_dir() {
                    drives.push((e.file_name().to_string_lossy().into_owned(), e.path().to_string_lossy().into_owned()));
                }
            }
        }
    }
    Json(serde_json::json!({ "home": home, "places": places, "drives": drives })).into_response()
}

pub async fn local_list(Query(q): Query<LocalQ>) -> Response {
    let p = PathBuf::from(q.path.unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| "/".into())));
    let res = blocking(move || {
        let rd = std::fs::read_dir(&p).map_err(|e| format!("{}: {e}", p.display()))?;
        let mut entries = Vec::new();
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let hidden = name.starts_with('.');
            if hidden && !q.hidden {
                continue;
            }
            let Ok(meta) = e.metadata() else { continue };
            let ext = e.path().extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
            let modified = meta.modified().ok().map(|t| chrono::DateTime::<chrono::Utc>::from(t).format("%Y-%m-%d %H:%M").to_string());
            entries.push(serde_json::json!({
                "name": name, "path": e.path(), "is_dir": meta.is_dir(),
                "size": if meta.is_dir() { 0 } else { meta.len() },
                "modified": modified, "ext": ext, "hidden": hidden,
                "kind": if meta.is_dir() { "folder" } else { kind_by_ext(&ext) },
            }));
        }
        entries.sort_by(|a, b| {
            b["is_dir"].as_bool().cmp(&a["is_dir"].as_bool()).then(
                a["name"].as_str().unwrap_or("").to_lowercase().cmp(&b["name"].as_str().unwrap_or("").to_lowercase()),
            )
        });
        Ok(serde_json::json!({ "path": p, "parent": p.parent(), "entries": entries }))
    })
    .await;
    match res {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

pub async fn local_read(Query(q): Query<LocalQ>) -> Response {
    let Some(p) = q.path.clone() else { return err(StatusCode::BAD_REQUEST, "?path= required".into()) };
    let res = blocking(move || {
        use std::io::{Read, Seek, SeekFrom};
        let mut f = std::fs::File::open(&p).map_err(|e| e.to_string())?;
        let size = f.metadata().map(|m| m.len()).unwrap_or(0);
        f.seek(SeekFrom::Start(q.offset)).map_err(|e| e.to_string())?;
        let want = if q.mode == "strings" { 4 << 20 } else { q.len.unwrap_or(4096).min(MAX_READ) };
        let mut data = Vec::new();
        f.take(want as u64).read_to_end(&mut data).map_err(|e| e.to_string())?;
        let mut v = if q.mode == "strings" {
            serde_json::json!({ "offset": q.offset, "strings": strings(&data, q.offset, 2000) })
        } else {
            dump_json(q.offset, &data)
        };
        v["size"] = size.into();
        v["kind"] = sniff(&data, &p).0.into();
        Ok(v)
    })
    .await;
    match res {
        Ok(v) => Json(v).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

pub async fn local_raw(Query(q): Query<LocalQ>) -> Response {
    let Some(p) = q.path.clone() else { return err(StatusCode::BAD_REQUEST, "?path= required".into()) };
    let size = std::fs::metadata(&p).map(|m| m.len()).unwrap_or(0);
    if size > MAX_RAW {
        return err(StatusCode::PAYLOAD_TOO_LARGE, format!("file is {size} bytes (preview limit {MAX_RAW})"));
    }
    match tokio::fs::read(&p).await {
        Ok(data) => bytes_response(data, &p),
        Err(e) => err(StatusCode::BAD_REQUEST, e.to_string()),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn bytes_response(data: Vec<u8>, name: &str) -> Response {
    let mime = sniff(&data[..data.len().min(4096)], name).1;
    let fname = Path::new(name).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let mut resp = Response::new(Body::from(data));
    if let Ok(v) = HeaderValue::from_str(mime) {
        resp.headers_mut().insert(header::CONTENT_TYPE, v);
    }
    if let Ok(v) = HeaderValue::from_str(&format!("inline; filename=\"{}\"", fname.replace('"', ""))) {
        resp.headers_mut().insert(header::CONTENT_DISPOSITION, v);
    }
    resp
}

/// (kind label, MIME type) from magic bytes, then the file name.
fn sniff(head: &[u8], name: &str) -> (&'static str, &'static str) {
    let starts = |m: &[u8]| head.starts_with(m);
    if starts(b"luo ") {
        return ("JUAN recording slot", "application/octet-stream");
    }
    if starts(&[0xFF, 0xD8, 0xFF]) {
        return ("JPEG image", "image/jpeg");
    }
    if starts(b"\x89PNG") {
        return ("PNG image", "image/png");
    }
    if starts(b"GIF8") {
        return ("GIF image", "image/gif");
    }
    if starts(b"%PDF") {
        return ("PDF document", "application/pdf");
    }
    if starts(b"SQLite format 3") {
        return ("SQLite database", "application/octet-stream");
    }
    if head.len() > 8 && &head[4..8] == b"ftyp" {
        return ("MP4 video", "video/mp4");
    }
    if starts(b"EVF\x09\x0d\x0a\xff\x00") {
        return ("E01 evidence segment", "application/octet-stream");
    }
    if starts(&[0, 0, 0, 1]) || starts(&[0, 0, 1]) {
        return ("video stream", "application/octet-stream");
    }
    if starts(b"\x7fELF") {
        return ("ELF executable", "application/octet-stream");
    }
    let ext = Path::new(name).extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
    match kind_by_ext(&ext) {
        "video" => return ("video", "video/mp4"),
        "picture" => return ("image", "image/jpeg"),
        _ => {}
    }
    if !head.is_empty() && head.iter().all(|&b| b == 0) {
        return ("empty (all zero bytes)", "application/octet-stream");
    }
    let printable = head.iter().filter(|&&b| b == b'\n' || b == b'\r' || b == b'\t' || (0x20..0x7f).contains(&b)).count();
    if !head.is_empty() && printable * 100 / head.len() > 90 {
        return ("text", "text/plain; charset=utf-8");
    }
    ("binary data", "application/octet-stream")
}

fn kind_by_ext(ext: &str) -> &'static str {
    match ext {
        "e01" | "ex01" => "evidence",
        "e02" | "e03" | "e04" | "e05" | "e06" | "e07" | "e08" | "e09" => "evidence segment",
        "dd" | "raw" | "img" | "001" | "bin" => "disk image",
        "mp4" | "mkv" | "webm" | "avi" | "mov" | "h264" | "hevc" | "h265" | "264" | "dav" => "video",
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" => "picture",
        "pdf" => "pdf",
        "txt" | "log" | "csv" | "json" | "md" | "xml" | "ini" | "cfg" => "text",
        "db" | "sqlite" => "database",
        "zip" | "7z" | "gz" | "tar" | "xz" | "rar" => "archive",
        _ => "file",
    }
}

pub(crate) fn hexdump(base: u64, data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len() * 4);
    for (i, chunk) in data.chunks(16).enumerate() {
        out.push_str(&format!("{:08x}  ", base + i as u64 * 16));
        for (k, b) in chunk.iter().enumerate() {
            out.push_str(&format!("{b:02x} "));
            if k == 7 {
                out.push(' ');
            }
        }
        for k in chunk.len()..16 {
            out.push_str("   ");
            if k == 7 {
                out.push(' ');
            }
        }
        out.push_str(" |");
        out.extend(chunk.iter().map(|&b| if (0x20..0x7f).contains(&b) { b as char } else { '.' }));
        out.push_str("|\n");
    }
    out
}

fn dump_json(offset: u64, data: &[u8]) -> serde_json::Value {
    serde_json::json!({
        "offset": offset,
        "len": data.len(),
        "hex": hexdump(offset, data),
        "text": String::from_utf8_lossy(data),
    })
}

/// Printable ASCII runs of 5+ characters with their offsets.
fn strings(data: &[u8], base: u64, max: usize) -> Vec<(u64, String)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, &b) in data.iter().enumerate() {
        let ok = (0x20..0x7f).contains(&b) || b == b'\t';
        match (ok, start) {
            (true, None) => start = Some(i),
            (false, Some(s)) => {
                if i - s >= 5 {
                    out.push((base + s as u64, String::from_utf8_lossy(&data[s..i]).into_owned()));
                    if out.len() >= max {
                        return out;
                    }
                }
                start = None;
            }
            _ => {}
        }
    }
    if let Some(s) = start {
        if data.len() - s >= 5 {
            out.push((base + s as u64, String::from_utf8_lossy(&data[s..]).into_owned()));
        }
    }
    out
}

fn count_pictures(stream: &[u8]) -> u64 {
    memchr::memmem::find_iter(stream, &[0, 0, 1])
        .filter(|&p| {
            let n = &stream[p + 3..];
            n.len() > 2 && (n[0] >> 1) & 0x3F <= 31 && n[2] & 0x80 != 0
        })
        .count() as u64
}

/// A relative path with no "..", root or drive components.
fn safe_rel(p: &str) -> PathBuf {
    p.split(['/', '\\'])
        .filter(|s| !s.is_empty() && *s != "." && *s != "..")
        .map(|s| s.replace(':', "_"))
        .collect()
}

pub(crate) fn urlenc(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}
