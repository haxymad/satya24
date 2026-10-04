//! AI provider settings (MCP tab): provider, model and API key.
//!
//! Stored in $XDG_CONFIG_HOME/satya/settings.json (or ~/.config/satya/),
//! outside the project so it never lands in git, with mode 0600 on Unix.
//! The full key is never sent back to the browser, only a masked hint.

use axum::{http::StatusCode, response::IntoResponse, Json};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Default, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub api_key: String,
}

fn path() -> PathBuf {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("satya").join("settings.json")
}

pub(crate) fn load() -> Settings {
    std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(s: &Settings) -> Result<PathBuf, String> {
    let p = path();
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d).map_err(|e| format!("create {}: {e}", d.display()))?;
    }
    let bytes = serde_json::to_vec_pretty(s).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&p)
            .map_err(|e| format!("write {}: {e}", p.display()))?;
        f.write_all(&bytes).map_err(|e| e.to_string())?;
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    std::fs::write(&p, &bytes).map_err(|e| e.to_string())?;
    Ok(p)
}

fn hint(key: &str) -> Option<String> {
    let k = key.trim();
    if k.is_empty() {
        return None;
    }
    let tail: String = k.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
    let head: String = k.chars().take(k.find('-').map(|i| i + 1).unwrap_or(0).min(8)).collect();
    Some(format!("{head}…{tail}"))
}

fn mcp_command() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|e| e.parent().map(|d| d.join(if cfg!(windows) { "satya-mcp.exe" } else { "satya-mcp" })))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "satya-mcp".into())
}

pub async fn get() -> impl IntoResponse {
    let s = load();
    Json(serde_json::json!({
        "provider": if s.provider.is_empty() { "anthropic" } else { s.provider.as_str() },
        "model": s.model,
        "key_set": !s.api_key.trim().is_empty(),
        "key_hint": hint(&s.api_key),
        "stored_at": path(),
        "mcp_command": mcp_command(),
    }))
}

#[derive(Deserialize)]
pub struct Update {
    provider: Option<String>,
    model: Option<String>,
    /// None = keep the current key; "" = remove it.
    api_key: Option<String>,
}

pub async fn put(Json(u): Json<Update>) -> impl IntoResponse {
    let mut s = load();
    if let Some(p) = u.provider {
        s.provider = p.trim().to_lowercase();
    }
    if let Some(m) = u.model {
        s.model = m.trim().to_string();
    }
    if let Some(k) = u.api_key {
        s.api_key = k.trim().to_string();
    }
    match save(&s) {
        Ok(p) => Json(serde_json::json!({ "saved": p, "key_set": !s.api_key.is_empty(), "key_hint": hint(&s.api_key) })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, e).into_response(),
    }
}

/// Check the stored key with a harmless "list models" call.
pub async fn test() -> impl IntoResponse {
    let s = load();
    if s.api_key.trim().is_empty() {
        return (StatusCode::BAD_REQUEST, "no API key saved".to_string()).into_response();
    }
    let client = match reqwest::Client::builder().timeout(std::time::Duration::from_secs(15)).build() {
        Ok(c) => c,
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    };
    let req = match s.provider.as_str() {
        "openai" => client.get("https://api.openai.com/v1/models").bearer_auth(s.api_key.trim()),
        _ => client
            .get("https://api.anthropic.com/v1/models")
            .header("x-api-key", s.api_key.trim())
            .header("anthropic-version", "2023-06-01"),
    };
    match req.send().await {
        Ok(r) if r.status().is_success() => {
            let v: serde_json::Value = r.json().await.unwrap_or_default();
            let models: Vec<String> = v["data"]
                .as_array()
                .map(|a| a.iter().filter_map(|m| m["id"].as_str().map(str::to_string)).take(30).collect())
                .unwrap_or_default();
            Json(serde_json::json!({ "ok": true, "models": models })).into_response()
        }
        Ok(r) => {
            let code = r.status();
            Json(serde_json::json!({ "ok": false, "error": format!("provider rejected the key (HTTP {code})") })).into_response()
        }
        Err(e) => Json(serde_json::json!({ "ok": false, "error": format!("could not reach provider: {e}") })).into_response(),
    }
}
