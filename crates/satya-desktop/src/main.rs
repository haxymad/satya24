//! SATYA desktop — Tauri native window with embedded API + auto ML.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

const BIND_ADDR: &str = "127.0.0.1:8000";
const ML_ADDR: &str = "127.0.0.1:8001";

struct ChildGuard(Arc<Mutex<Option<Child>>>);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if let Ok(mut g) = self.0.lock() {
            if let Some(mut child) = g.take() {
                eprintln!("[satya] stopping ML sidecar…");
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

fn find_dir(name: &str) -> Option<PathBuf> {
    if let Ok(repo) = std::env::var("SATYA_REPO") {
        let c = PathBuf::from(&repo).join(name);
        if c.exists() { return Some(c); }
    }
    if let Ok(cwd) = std::env::current_dir() {
        let c = cwd.join(name);
        if c.exists() { return Some(c); }
    }
    if let Ok(exe) = std::env::current_exe() {
        for ancestor in exe.ancestors() {
            let c = ancestor.join(name);
            if c.exists() { return Some(c); }
        }
    }
    None
}

fn which_python() -> Option<String> {
    for name in ["python3", "python"] {
        if Command::new(name)
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok()
        {
            return Some(name.into());
        }
    }
    None
}

fn spawn_ml_sidecar() -> ChildGuard {
    let handle = Arc::new(Mutex::new(None));

    let python_dir = match find_dir("python") {
        Some(p) => p,
        None => {
            eprintln!("[satya] python/ not found — ML disabled");
            return ChildGuard(handle);
        }
    };
    let models_dir = find_dir("models")
        .unwrap_or_else(|| python_dir.join("..").join("models"));
    let yolo = models_dir.join("yolov8n.onnx");

    let python = match which_python() {
        Some(p) => p,
        None => {
            eprintln!("[satya] python3 not found — ML disabled");
            return ChildGuard(handle);
        }
    };

    eprintln!("[satya] python dir: {}", python_dir.display());
    eprintln!("[satya] models dir: {}", models_dir.display());

    let mut cmd = Command::new(&python);
    cmd.args(["-m", "dvr_forensics.ml_server"])
        .env("PYTHONPATH", python_dir.to_string_lossy().to_string())
        .env("SATYA_YOLO_MODEL", yolo.to_string_lossy().to_string())
        .env("SATYA_FACE_DETECT",
             models_dir.join("retinaface.onnx").to_string_lossy().to_string())
        .env("SATYA_FACE_EMBED",
             models_dir.join("arcface.onnx").to_string_lossy().to_string())
        .env("SATYA_ML_BIND", ML_ADDR)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());

    match cmd.spawn() {
        Ok(child) => {
            eprintln!("[satya] ML sidecar pid {}", child.id());
            *handle.lock().unwrap() = Some(child);
        }
        Err(e) => {
            eprintln!("[satya] failed to start ML sidecar: {e}");
        }
    }
    ChildGuard(handle)
}

fn start_api() {
    thread::spawn(|| {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        rt.block_on(async {
            if let Err(e) = satya_api::run(BIND_ADDR).await {
                eprintln!("[satya] API error: {e}");
            }
        });
    });
}

fn wait_for_api(timeout_ms: u64) -> bool {
    let start = Instant::now();
    let addr: std::net::SocketAddr = BIND_ADDR.parse().unwrap();
    while start.elapsed().as_millis() < timeout_ms as u128 {
        if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(100)).is_ok() {
            return true;
        }
        thread::sleep(Duration::from_millis(50));
    }
    false
}

#[tauri::command]
async fn repo_info() -> serde_json::Value {
    serde_json::json!({
        "cwd": std::env::current_dir().map(|p| p.to_string_lossy().to_string()).ok(),
        "api": BIND_ADDR,
        "ml": ML_ADDR,
    })
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "satya_api=info,tower_http=warn".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    // Start ML sidecar first
    let no_ml = std::env::var("SATYA_NO_ML").ok().as_deref() == Some("1");
    let _ml = if no_ml { None } else { Some(spawn_ml_sidecar()) };

    // Start API on a background thread
    start_api();

    // Give the API time to bind (Tauri will load the URL)
    if !wait_for_api(8000) {
        eprintln!("[satya] warning: API did not bind within 8s");
    } else {
        eprintln!("[satya] API ready at http://{}", BIND_ADDR);
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .invoke_handler(tauri::generate_handler![repo_info])
        .setup(|_app| {
            eprintln!("[satya] window opened");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error running Tauri app");
}
