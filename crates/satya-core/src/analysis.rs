use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DeviceInfo {
    pub oem: String,
    pub model: Option<String>,
    pub firmware: Option<String>,
    pub serial: Option<String>,
    pub mac: Option<String>,
    pub uuid: Option<String>,
    pub block_size: Option<u32>,
    pub sector_size: Option<u32>,
    pub total_bytes: u64,
    pub disk_size_gb: f64,
    pub channels: Option<u32>,
    pub manufacturer_string: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Partition {
    pub index: u32,
    pub name: String,
    pub offset_bytes: u64,
    pub size_bytes: u64,
    pub fs_type: String,
    pub role: String,          // "system", "video", "log", "reserved"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub id: u32,
    pub label: String,
    pub frame_count: usize,
    pub total_bytes: u64,
    pub first_offset: u64,
    pub last_offset: u64,
    pub resolution: Option<String>,
    pub codec: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogRecord {
    pub offset: u64,
    pub timestamp_utc: Option<String>,
    pub major_type: u16,
    pub minor_type: u16,
    pub category: String,       // decoded human-readable type
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationRegion {
    pub start: u64,
    pub end: u64,
    pub kind: String,          // "allocated" | "unallocated" | "system" | "log"
    pub frame_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoStats {
    pub total_frames: usize,
    pub allocated: usize,
    pub carved: usize,
    pub partial: usize,
    pub h264: usize,
    pub h265: usize,
    pub key_frames: usize,
    pub total_video_bytes: u64,
    pub bitrate_kbps: Option<f64>,
    pub duration_seconds: Option<f64>,
    pub fps_estimate: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnalysisSummary {
    pub device: DeviceInfo,
    pub partitions: Vec<Partition>,
    pub channels: Vec<Channel>,
    pub logs: Vec<LogRecord>,
    pub allocation: Vec<AllocationRegion>,
    pub stats: VideoStats,
}
