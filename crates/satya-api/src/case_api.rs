//! Case actions behind the menu bar, plus the session activity log.
//!
//! Every significant action (analyze, export, extract, verify, report, save)
//! is appended to a hash chain and signed with an Ed25519 key created when
//! the app starts. The Custody tab and the PDF report show this log. The key
//! lives only for the session; its public half is printed in the report.

use super::AppState;
use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use satya_custody::{checkpoint, CustodySigner, GENESIS_HASH};
use serde::Serialize;
use std::sync::{Mutex, OnceLock};

fn signer() -> &'static CustodySigner {
    static S: OnceLock<CustodySigner> = OnceLock::new();
    S.get_or_init(CustodySigner::generate)
}

pub(crate) fn session_public_key() -> String {
    signer().verifying_key_hex()
}

/// Append an event to the hash-chained, signed activity log.
pub(crate) async fn log_event(state: &AppState, event: serde_json::Value) {
    let mut inner = state.inner.lock().await;
    let prev = inner
        .custody
        .last()
        .and_then(|c| c["hash"].as_str().map(str::to_string))
        .unwrap_or_else(|| GENESIS_HASH.to_string());
    let cp = checkpoint(&prev, event, signer());
    if let Ok(v) = serde_json::to_value(&cp) {
        inner.custody.push(v);
    }
}

fn stamp() -> String {
    chrono::Utc::now().format("%Y%m%d_%H%M%S").to_string()
}

fn out_dir() -> Result<std::path::PathBuf, String> {
    let d = crate::juan_api::out_dir();
    std::fs::create_dir_all(&d).map_err(|e| format!("create {}: {e}", d.display()))?;
    Ok(d)
}

// ---------------------------------------------------------------------------
// Save / close
// ---------------------------------------------------------------------------

pub async fn save_case(State(state): State<AppState>) -> impl IntoResponse {
    let doc = {
        let i = state.inner.lock().await;
        if i.image_path.is_empty() {
            return (StatusCode::BAD_REQUEST, "no case loaded".to_string()).into_response();
        }
        let parse = |s: &Option<String>| s.as_deref().and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok());
        serde_json::json!({
            "tool": format!("satya-api {}", env!("CARGO_PKG_VERSION")),
            "saved_utc": chrono::Utc::now(),
            "case": {
                "image_path": i.image_path, "oem": i.oem, "confidence": i.confidence,
                "md5": i.md5, "sha256": i.sha256, "size_bytes": i.size_bytes,
                "total_frames": i.total_frames, "allocated": i.allocated, "carved": i.carved,
                "video_path": i.video_path,
            },
            "summary": parse(&i.summary_json),
            "frames": parse(&i.frames_json),
            "timeline": parse(&i.timeline_json),
            "activity_log": i.custody,
            "session_public_key": session_public_key(),
        })
    };
    let res = (|| -> Result<serde_json::Value, String> {
        let path = out_dir()?.join(format!("case_{}.json", stamp()));
        let bytes = serde_json::to_vec_pretty(&doc).map_err(|e| e.to_string())?;
        std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
        use sha2::Digest;
        Ok(serde_json::json!({ "path": path, "bytes": bytes.len(), "sha256": hex::encode(sha2::Sha256::digest(&bytes)) }))
    })();
    match res {
        Ok(v) => {
            super::log_event(&state, serde_json::json!({"action": "save_case", "file": v["path"], "sha256": v["sha256"]})).await;
            Json(v).into_response()
        }
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

pub async fn close_case(State(state): State<AppState>) -> impl IntoResponse {
    let mut i = state.inner.lock().await;
    *i = Default::default();
    Json(serde_json::json!({ "closed": true }))
}

/// Frames (or JUAN slots) with their fused timestamps, as CSV.
pub async fn export_csv(State(state): State<AppState>) -> impl IntoResponse {
    let (frames, timeline) = {
        let i = state.inner.lock().await;
        (i.frames_json.clone(), i.timeline_json.clone())
    };
    let Some(frames) = frames else {
        return (StatusCode::BAD_REQUEST, "no case loaded".to_string()).into_response();
    };
    let frames: Vec<serde_json::Value> = serde_json::from_str(&frames).unwrap_or_default();
    let tl: Vec<serde_json::Value> = timeline.as_deref().and_then(|j| serde_json::from_str(j).ok()).unwrap_or_default();
    let mut csv = String::from("index,offset,sector,length,codec,source,confidence,claimed_utc,posterior_utc,ci95_lo_utc,ci95_hi_utc\n");
    let iso = |v: &serde_json::Value| {
        v.as_f64().and_then(|s| chrono::DateTime::from_timestamp(s as i64, 0)).map(|d| d.to_rfc3339()).unwrap_or_default()
    };
    for (i, f) in frames.iter().enumerate() {
        let off = f["offset"].as_u64().unwrap_or(0);
        let t = tl.get(i);
        csv.push_str(&format!(
            "{i},{off},{},{},{},{},{},{},{},{},{}\n",
            off / 512,
            f["length"],
            f["codec"].as_str().unwrap_or(""),
            f["recovery_source"].as_str().unwrap_or(""),
            f["recovery_confidence"],
            f["claims"][0]["claimed_utc"].as_str().unwrap_or(""),
            t.map(|t| iso(&t["mean_s"])).unwrap_or_default(),
            t.map(|t| iso(&t["ci_lo_95_s"])).unwrap_or_default(),
            t.map(|t| iso(&t["ci_hi_95_s"])).unwrap_or_default(),
        ));
    }
    let res = out_dir().and_then(|d| {
        let p = d.join(format!("frames_{}.csv", stamp()));
        std::fs::write(&p, csv.as_bytes()).map_err(|e| e.to_string()).map(|_| p)
    });
    match res {
        Ok(p) => Json(serde_json::json!({ "path": p, "rows": frames.len() })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

// ---------------------------------------------------------------------------
// Full-media verification (background, with progress)
// ---------------------------------------------------------------------------

#[derive(Clone, Default, Serialize)]
pub struct VerifyJob {
    pub status: String,
    pub image_path: String,
    pub done: u64,
    pub total: u64,
    pub started_utc: Option<chrono::DateTime<chrono::Utc>>,
    pub finished_utc: Option<chrono::DateTime<chrono::Utc>>,
    pub result: Option<satya_image::hashing::MediaHashes>,
    pub error: Option<String>,
}

pub(crate) fn verify_job() -> &'static Mutex<VerifyJob> {
    static J: OnceLock<Mutex<VerifyJob>> = OnceLock::new();
    J.get_or_init(|| Mutex::new(VerifyJob { status: "idle".into(), ..Default::default() }))
}

/// Finished verification for `image`, if one was run this session.
pub(crate) fn verification_for(image: &str) -> Option<satya_image::hashing::MediaHashes> {
    let j = verify_job().lock().unwrap();
    (j.status == "done" && j.image_path == image).then(|| j.result.clone()).flatten()
}

pub async fn verify_start(State(state): State<AppState>) -> impl IntoResponse {
    let image = state.inner.lock().await.image_path.clone();
    if image.is_empty() {
        return (StatusCode::BAD_REQUEST, "no case loaded".to_string()).into_response();
    }
    {
        let mut j = verify_job().lock().unwrap();
        if j.status == "running" {
            return (StatusCode::CONFLICT, "verification already running".to_string()).into_response();
        }
        *j = VerifyJob { status: "running".into(), image_path: image.clone(), started_utc: Some(chrono::Utc::now()), ..Default::default() };
    }
    let st = state.clone();
    let rt = tokio::runtime::Handle::current();
    std::thread::spawn(move || {
        let res = satya_image::open_image(std::path::Path::new(&image))
            .and_then(|src| {
                satya_image::hashing::hash_media(src.as_ref(), |d, t| {
                    let mut j = verify_job().lock().unwrap();
                    j.done = d;
                    j.total = t;
                })
            });
        let event = {
            let mut j = verify_job().lock().unwrap();
            j.finished_utc = Some(chrono::Utc::now());
            match res {
                Ok(h) => {
                    j.status = "done".into();
                    let ev = serde_json::json!({
                        "action": "verify", "md5": h.md5, "sha256": h.sha256,
                        "md5_matches_stored": h.md5_matches, "sha1_matches_stored": h.sha1_matches,
                    });
                    j.result = Some(h);
                    ev
                }
                Err(e) => {
                    j.status = "error".into();
                    j.error = Some(e.to_string());
                    serde_json::json!({ "action": "verify", "error": e.to_string() })
                }
            }
        };
        rt.block_on(async {
            let current = st.inner.lock().await.image_path.clone();
            if let Some(h) = verification_for(&current) {
                st.inner.lock().await.sha256 = h.sha256.clone();
            }
            super::log_event(&st, event).await;
        });
    });
    (StatusCode::ACCEPTED, Json(verify_job().lock().unwrap().clone())).into_response()
}

pub async fn verify_status() -> impl IntoResponse {
    Json(verify_job().lock().unwrap().clone())
}

// ---------------------------------------------------------------------------
// Output folder listing
// ---------------------------------------------------------------------------

pub async fn outputs() -> impl IntoResponse {
    let dir = crate::juan_api::out_dir();
    let mut files = Vec::new();
    let mut stack = vec![(dir.clone(), 0)];
    while let Some((d, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                if depth < 3 {
                    stack.push((e.path(), depth + 1));
                }
                continue;
            }
            let modified = m.modified().ok().map(|t| chrono::DateTime::<chrono::Utc>::from(t).to_rfc3339());
            files.push(serde_json::json!({
                "path": e.path(), "name": e.path().strip_prefix(&dir).unwrap_or(&e.path()).to_string_lossy(),
                "bytes": m.len(), "modified": modified,
            }));
        }
    }
    files.sort_by(|a, b| b["modified"].as_str().cmp(&a["modified"].as_str()));
    Json(serde_json::json!({ "dir": dir, "files": files }))
}
