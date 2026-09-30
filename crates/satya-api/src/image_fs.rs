//! Virtual filesystem exposed by the DVR image.
//!
//! Paths:
//!   /                       root
//!   /device                 device info (JSON)
//!   /partitions             dir of partitions
//!   /partitions/N           partition N (JSON)
//!   /logs                   dir of log records
//!   /logs/N                 log record N (JSON)
//!   /frames                 dir (allocated / carved)
//!   /frames/allocated       dir
//!   /frames/carved          dir
//!   /frames/allocated/N     frame N metadata (JSON) + hex
//!   /frames/carved/N        frame N metadata (JSON) + hex
//!   /channels               dir
//!   /channels/N             channel N (JSON)
//!   /unallocated            dir of regions
//!   /unallocated/N          region N (JSON) + hex

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct FsNode {
    pub name: String,
    pub path: String,
    pub kind: String,       // "dir" | "json" | "hex"
    pub size: Option<u64>,
    pub detail: Option<String>,
}

pub struct ImageFs<'a> {
    pub summary: &'a serde_json::Value,
    pub frames: &'a [serde_json::Value],
    #[allow(dead_code)]
    pub image_size: u64,
}

impl<'a> ImageFs<'a> {
    pub fn new(
        summary: &'a serde_json::Value,
        frames: &'a [serde_json::Value],
        image_size: u64,
    ) -> Self {
        Self { summary, frames, image_size }
    }

    /// List children of the given path. Paths must start with `/`.
    pub fn list(&self, path: &str) -> Result<Vec<FsNode>, String> {
        let path = path.trim_end_matches('/');
        if path.is_empty() || path == "/" || path == "" {
            return Ok(self.root());
        }
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        match parts.as_slice() {
            ["device"] => Ok(vec![]),
            ["partitions"] => Ok(self.partitions()),
            [p] if p.starts_with("partitions") => Err("not a directory".into()),
            ["logs"] => Ok(self.logs()),
            ["frames"] => Ok(self.frames_root()),
            ["frames", group] => Ok(self.frames_group(group)),
            ["channels"] => Ok(self.channels()),
            ["unallocated"] => Ok(self.unallocated()),
            _ => Err(format!("unknown path: {path}")),
        }
    }

    /// Return the hex dump for a leaf node.
    pub fn hex(&self, path: &str, max_bytes: usize) -> Result<String, String> {
        let parts: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        let (offset, length) = match parts.as_slice() {
            ["partitions", n] => {
                let p = self.summary["partitions"].get(parse_idx(n)?)
                    .ok_or_else(|| "partition index out of range".to_string())?;
                (
                    p["offset_bytes"].as_u64().unwrap_or(0),
                    p["size_bytes"].as_u64().unwrap_or(512).min(4096),
                )
            }
            ["logs", n] => {
                let log = self.summary["logs"].get(parse_idx(n)?)
                    .ok_or_else(|| "log index out of range".to_string())?;
                (
                    log["offset"].as_u64().unwrap_or(0),
                    128,
                )
            }
            ["frames", group, n] => {
                let idx = parse_idx(n)?;
                let target_group = if *group == "allocated" { "Allocated" } else { "Carved" };
                let filtered: Vec<&serde_json::Value> = self.frames.iter()
                    .filter(|f| f["recovery_source"].as_str().unwrap_or("") == target_group)
                    .collect();
                let f = filtered.get(idx)
                    .ok_or_else(|| format!("frame {idx} not in group {group}"))?;
                (
                    f["offset"].as_u64().unwrap_or(0),
                    (f["length"].as_u64().unwrap_or(256) as usize).min(max_bytes) as u64,
                )
            }
            ["unallocated", n] => {
                let region = self.summary["allocation"].get(parse_idx(n)?)
                    .ok_or_else(|| "region index out of range".to_string())?;
                if region["kind"].as_str() != Some("unallocated") {
                    return Err("not an unallocated region".into());
                }
                (
                    region["start"].as_u64().unwrap_or(0),
                    (region["end"].as_u64().unwrap_or(256) - region["start"].as_u64().unwrap_or(0))
                        .min(max_bytes as u64),
                )
            }
            _ => return Err(format!("no hex for path: {path}")),
        };
        Ok(format!("0x{:x}..0x{:x}", offset, offset + length))
    }

    pub fn byte_range(&self, path: &str, max_bytes: usize) -> Result<(u64, u64), String> {
        let s = self.hex(path, max_bytes)?;
        let cleaned = s.trim_start_matches("0x");
        let (a, b) = cleaned.split_once("..0x").ok_or("bad range")?;
        Ok((u64::from_str_radix(a, 16).map_err(|e| e.to_string())?,
            u64::from_str_radix(b, 16).map_err(|e| e.to_string())?))
    }

    // -- node builders ------------------------------------------------------

    fn root(&self) -> Vec<FsNode> {
        vec![
            dir("/", "device", "device info"),
            dir("/", "partitions", "disk partitions"),
            dir("/", "logs", "device log records"),
            dir("/", "frames", "video frames"),
            dir("/", "channels", "camera channels"),
            dir("/", "unallocated", "unallocated regions"),
        ]
    }

    fn partitions(&self) -> Vec<FsNode> {
        let arr = self.summary["partitions"].as_array();
        match arr {
            Some(list) => list.iter().enumerate().map(|(i, p)| FsNode {
                name: format!("[{}] {}", i, p["name"].as_str().unwrap_or("?")),
                path: format!("/partitions/{i}"),
                kind: "hex".into(),
                size: p["size_bytes"].as_u64(),
                detail: Some(format!(
                    "{} · {}",
                    p["fs_type"].as_str().unwrap_or(""),
                    p["role"].as_str().unwrap_or("")
                )),
            }).collect(),
            None => vec![],
        }
    }

    fn logs(&self) -> Vec<FsNode> {
        let arr = self.summary["logs"].as_array();
        match arr {
            Some(list) => list.iter().take(500).enumerate().map(|(i, l)| FsNode {
                name: format!("[{}] {}", i, l["category"].as_str().unwrap_or("log")),
                path: format!("/logs/{i}"),
                kind: "hex".into(),
                size: Some(128),
                detail: Some(format!(
                    "{} · {}",
                    l["timestamp_utc"].as_str().unwrap_or("no timestamp"),
                    l["description"].as_str().unwrap_or("")
                )),
            }).collect(),
            None => vec![],
        }
    }

    fn frames_root(&self) -> Vec<FsNode> {
        let allocated = self.frames.iter()
            .filter(|f| f["recovery_source"].as_str() == Some("Allocated")).count();
        let carved = self.frames.iter()
            .filter(|f| f["recovery_source"].as_str() == Some("Carved")).count();
        vec![
            FsNode {
                name: format!("allocated ({allocated})"),
                path: "/frames/allocated".into(),
                kind: "dir".into(),
                size: None, detail: None,
            },
            FsNode {
                name: format!("carved ({carved})"),
                path: "/frames/carved".into(),
                kind: "dir".into(),
                size: None, detail: None,
            },
        ]
    }

    fn frames_group(&self, group: &str) -> Vec<FsNode> {
        let target = if group == "allocated" { "Allocated" } else { "Carved" };
        self.frames.iter()
            .filter(|f| f["recovery_source"].as_str() == Some(target))
            .take(500)
            .enumerate()
            .map(|(i, f)| FsNode {
                name: format!("frame_{:06}.h264", i),
                path: format!("/frames/{group}/{i}"),
                kind: "hex".into(),
                size: Some(f["length"].as_u64().unwrap_or(0)),
                detail: Some(format!(
                    "offset=0x{:x} len={} conf={:.2}",
                    f["offset"].as_u64().unwrap_or(0),
                    f["length"].as_u64().unwrap_or(0),
                    f["recovery_confidence"].as_f64().unwrap_or(0.0),
                )),
            })
            .collect()
    }

    fn channels(&self) -> Vec<FsNode> {
        let arr = self.summary["channels"].as_array();
        match arr {
            Some(list) => list.iter().enumerate().map(|(i, c)| FsNode {
                name: c["label"].as_str().unwrap_or(&format!("channel {i}")).to_string(),
                path: format!("/channels/{i}"),
                kind: "json".into(),
                size: None,
                detail: Some(format!(
                    "{} frames · {} bytes",
                    c["frame_count"].as_u64().unwrap_or(0),
                    c["total_bytes"].as_u64().unwrap_or(0),
                )),
            }).collect(),
            None => vec![],
        }
    }

    fn unallocated(&self) -> Vec<FsNode> {
        let arr = self.summary["allocation"].as_array();
        match arr {
            Some(list) => list.iter()
                .filter(|r| r["kind"].as_str() == Some("unallocated"))
                .enumerate()
                .map(|(i, r)| FsNode {
                    name: format!("unalloc_{:04}.bin", i),
                    path: format!("/unallocated/{i}"),
                    kind: "hex".into(),
                    size: Some(r["end"].as_u64().unwrap_or(0) - r["start"].as_u64().unwrap_or(0)),
                    detail: Some(format!(
                        "0x{:x}..0x{:x}",
                        r["start"].as_u64().unwrap_or(0),
                        r["end"].as_u64().unwrap_or(0),
                    )),
                })
                .collect(),
            None => vec![],
        }
    }
}

fn dir(_base: &str, name: &str, detail: &str) -> FsNode {
    FsNode {
        name: name.into(),
        path: format!("/{name}"),
        kind: "dir".into(),
        size: None,
        detail: Some(detail.into()),
    }
}

fn parse_idx(s: &str) -> Result<usize, String> {
    s.parse().map_err(|_| format!("bad index: {s}"))
}
