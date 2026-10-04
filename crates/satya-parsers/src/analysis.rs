//! High-level extraction of DVR metadata beyond frame lists.

use satya_core::analysis::*;
use satya_core::*;

// ---------------------------------------------------------------------------
// Device info
// ---------------------------------------------------------------------------

pub fn extract_device_info(image: &[u8], fp: &DeviceFingerprint) -> DeviceInfo {
    let mut info = DeviceInfo {
        oem: fp.oem.as_str().to_string(),
        model: fp.model.clone(),
        firmware: fp.firmware.clone(),
        block_size: fp.block_size,
        sector_size: Some(512),
        total_bytes: image.len() as u64,
        disk_size_gb: image.len() as f64 / 1e9,
        ..Default::default()
    };

    // Try to extract strings from the first 8 KB — serial numbers,
    // model names, firmware versions often appear in plaintext.
    let head = &image[..image.len().min(8192)];
    if let Some(s) = find_ascii_string(head, b"DS-", 3) {
        info.model = Some(s);
    }
    if let Some(s) = find_ascii_string(head, b"V", 5) {
        if s.chars().filter(|c| c.is_ascii_digit()).count() > 3
            && s.contains('.') && info.firmware.is_none()
        {
            info.firmware = Some(s);
        }
    }
    // Serial: typically a long alphanumeric run following a known prefix
    let prefixes: [&[u8]; 4] = [b"SN:", b"S/N:", b"serial:", b"Serial:"];
    for prefix in prefixes {
        if let Some(pos) = find_subslice(head, prefix) {
            let start = pos + prefix.len();
            let s = read_alnum(&head[start..], 32);
            if s.len() >= 8 { info.serial = Some(s); break; }
        }
    }
    // UUID pattern in Hikvision/Dahua superblocks
    if fp.oem == Oem::Dahua {
        if let Some(u) = read_uuid(&image[..image.len().min(256)]) {
            info.uuid = Some(u);
        }
    }
    info.manufacturer_string = match fp.oem {
        Oem::Hikvision => Some("Hangzhou Hikvision Digital Technology Co., Ltd.".into()),
        Oem::Dahua => Some("Zhejiang Dahua Technology Co., Ltd.".into()),
        Oem::Juan => Some("JUAN (OEM for HeimVision / Zosi / Hiseeu / Kogan)".into()),
        _ => None,
    };
    info
}

// ---------------------------------------------------------------------------
// Partitions
// ---------------------------------------------------------------------------

pub fn extract_partitions(image: &[u8], oem: Oem) -> Vec<Partition> {
    let mut parts = Vec::new();
    match oem {
        Oem::Dahua => {
            parts.push(Partition {
                index: 0, name: "reserved".into(),
                offset_bytes: 0, size_bytes: 34 * 512,
                fs_type: "DHFS4.1 header".into(), role: "system".into(),
            });
            parts.push(Partition {
                index: 1, name: "video".into(),
                offset_bytes: 0x10000,
                size_bytes: (image.len() as u64).saturating_sub(0x10000),
                fs_type: "DHFS4.1".into(), role: "video".into(),
            });
        }
        Oem::Hikvision => {
            let log_off = u64::from_le_bytes(
                image.get(0x260..0x268)
                    .map(|s| s.try_into().unwrap()).unwrap_or([0; 8]));
            parts.push(Partition {
                index: 0, name: "master".into(),
                offset_bytes: 0, size_bytes: 2048,
                fs_type: "HVFS master".into(), role: "system".into(),
            });
            parts.push(Partition {
                index: 1, name: "log".into(),
                offset_bytes: log_off, size_bytes: 0,
                fs_type: "HVFS log".into(), role: "log".into(),
            });
            parts.push(Partition {
                index: 2, name: "video".into(),
                offset_bytes: 0x100000,
                size_bytes: (image.len() as u64).saturating_sub(0x100000),
                fs_type: "HVFS".into(), role: "video".into(),
            });
        }
        _ => {
            parts.push(Partition {
                index: 0, name: "full".into(),
                offset_bytes: 0, size_bytes: image.len() as u64,
                fs_type: oem.as_str().to_string(), role: "video".into(),
            });
        }
    }
    parts
}

// ---------------------------------------------------------------------------
// Channels — group frames by channel derived from header bytes
// ---------------------------------------------------------------------------

pub fn extract_channels(frames: &[RecoveredFrame], image: &[u8], oem: Oem) -> Vec<Channel> {
    use std::collections::BTreeMap;
    let mut buckets: BTreeMap<u32, Vec<&RecoveredFrame>> = BTreeMap::new();

    for f in frames {
        let ch = channel_of(image, f.offset as usize, oem);
        buckets.entry(ch).or_default().push(f);
    }

    buckets.into_iter().map(|(id, fs)| {
        let total: u64 = fs.iter().map(|f| f.length as u64).sum();
        Channel {
            id,
            label: format!("Camera {}", id + 1),
            frame_count: fs.len(),
            total_bytes: total,
            first_offset: fs.iter().map(|f| f.offset).min().unwrap_or(0),
            last_offset: fs.iter().map(|f| f.offset).max().unwrap_or(0),
            resolution: None,
            codec: "H264".into(),
        }
    }).collect()
}

fn channel_of(image: &[u8], offset: usize, oem: Oem) -> u32 {
    match oem {
        Oem::Dahua => {
            if offset + 6 < image.len() && &image[offset..offset + 4] == b"DHAV" {
                (image[offset + 5] as u32).wrapping_add(1)
            } else { 0 }
        }
        Oem::Hikvision => {
            if offset + 16 < image.len() && &image[offset..offset + 4] == b"RATS" {
                u16::from_le_bytes([image[offset + 14], image[offset + 15]]) as u32
            } else { 0 }
        }
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// Logs — decode known categories
// ---------------------------------------------------------------------------

pub fn extract_logs(image: &[u8], oem: Oem) -> Vec<LogRecord> {
    match oem {
        Oem::Hikvision => extract_hikvision_logs(image),
        _ => Vec::new(),
    }
}

fn extract_hikvision_logs(image: &[u8]) -> Vec<LogRecord> {
    let magic: &[u8] = &[0x52, 0x41, 0x54, 0x53, 0x01, 0x00, 0x00, 0x00];
    let mut logs = Vec::new();
    for pos in memchr::memmem::find_iter(image, magic) {
        if pos + 16 > image.len() { continue; }
        let ts = u32::from_le_bytes(image[pos + 8..pos + 12].try_into().unwrap());
        let major = u16::from_le_bytes(image[pos + 12..pos + 14].try_into().unwrap());
        let minor = u16::from_le_bytes(image[pos + 14..pos + 16].try_into().unwrap());
        let (cat, desc) = decode_hik_log(major, minor);
        logs.push(LogRecord {
            offset: pos as u64,
            timestamp_utc: chrono::DateTime::from_timestamp(ts as i64, 0)
                .map(|d| d.to_rfc3339()),
            major_type: major, minor_type: minor,
            category: cat.into(), description: desc.into(),
        });
        if logs.len() >= 5000 { break; }
    }
    logs
}

fn decode_hik_log(major: u16, minor: u16) -> (&'static str, &'static str) {
    match (major, minor) {
        (0x0001, 0x0001) => ("alarm", "Alarm triggered"),
        (0x0001, 0x0002) => ("alarm", "Motion detected"),
        (0x0002, 0x0001) => ("video", "Recording started"),
        (0x0002, 0x0002) => ("video", "Recording stopped"),
        (0x0003, 0x0001) => ("operation", "Login"),
        (0x0003, 0x0002) => ("operation", "Logout"),
        (0x0003, 0x0010) => ("operation", "NTP sync"),
        (0x0004, 0x0001) => ("system", "Boot"),
        (0x0004, 0x0002) => ("system", "Shutdown"),
        _ => ("unknown", "Unclassified record"),
    }
}

// ---------------------------------------------------------------------------
// Allocation map
// ---------------------------------------------------------------------------

pub fn extract_allocation(
    image: &[u8],
    frames: &[RecoveredFrame],
) -> Vec<AllocationRegion> {
    let mut regions = Vec::new();
    let mut allocated: Vec<(u64, u64)> = frames.iter()
        .filter(|f| matches!(f.recovery_source, RecoverySource::Allocated))
        .filter(|f| f.length > 0)
        .map(|f| (f.offset, f.offset + f.length as u64))
        .collect();
    allocated.sort_by_key(|r| r.0);

    // Merge overlapping/adjacent
    let mut merged: Vec<(u64, u64)> = Vec::new();
    for (s, e) in allocated {
        if let Some(last) = merged.last_mut() {
            if s <= last.1 { last.1 = last.1.max(e); continue; }
        }
        merged.push((s, e));
    }

    // Build alternating regions
    let total = image.len() as u64;
    let mut cursor = 0u64;
    for (s, e) in &merged {
        if *s > cursor {
            regions.push(AllocationRegion {
                start: cursor, end: *s,
                kind: "unallocated".into(), frame_count: 0,
            });
        }
        let count = frames.iter()
            .filter(|f| f.offset >= *s && f.offset < *e).count();
        regions.push(AllocationRegion {
            start: *s, end: *e,
            kind: "allocated".into(), frame_count: count,
        });
        cursor = *e;
    }
    if cursor < total {
        regions.push(AllocationRegion {
            start: cursor, end: total,
            kind: "unallocated".into(), frame_count: 0,
        });
    }
    regions
}

// ---------------------------------------------------------------------------
// Video stats
// ---------------------------------------------------------------------------

pub fn compute_stats(frames: &[RecoveredFrame]) -> VideoStats {
    let mut s = VideoStats {
        total_frames: frames.len(),
        allocated: 0, carved: 0, partial: 0,
        h264: 0, h265: 0, key_frames: 0,
        total_video_bytes: 0,
        bitrate_kbps: None, duration_seconds: None, fps_estimate: None,
    };
    for f in frames {
        match f.recovery_source {
            RecoverySource::Allocated => s.allocated += 1,
            RecoverySource::Carved => s.carved += 1,
            RecoverySource::Partial => s.partial += 1,
        }
        match f.codec { Codec::H264 => s.h264 += 1, Codec::H265 => s.h265 += 1 }
        if f.recovery_confidence > 0.9 { s.key_frames += 1; }
        s.total_video_bytes += f.length as u64;
    }
    // Duration estimate from frame count assuming 25 fps if timestamps absent
    if s.total_frames > 0 {
        s.duration_seconds = Some(s.total_frames as f64 / 25.0);
        s.fps_estimate = Some(25.0);
    }
    if let Some(d) = s.duration_seconds {
        if d > 0.0 {
            s.bitrate_kbps = Some((s.total_video_bytes as f64 * 8.0) / d / 1000.0);
        }
    }
    s
}

// ---------------------------------------------------------------------------
// Summary builder
// ---------------------------------------------------------------------------

pub fn build_summary(
    image: &[u8],
    fp: &DeviceFingerprint,
    frames: &[RecoveredFrame],
) -> AnalysisSummary {
    AnalysisSummary {
        device: extract_device_info(image, fp),
        partitions: extract_partitions(image, fp.oem),
        channels: extract_channels(frames, image, fp.oem),
        logs: extract_logs(image, fp.oem),
        allocation: extract_allocation(image, frames),
        stats: compute_stats(frames),
    }
}

// ---------------------------------------------------------------------------
// String helpers
// ---------------------------------------------------------------------------

fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
    memchr::memmem::find(hay, needle)
}

fn find_ascii_string(hay: &[u8], prefix: &[u8], max_len: usize) -> Option<String> {
    let pos = find_subslice(hay, prefix)?;
    let tail = &hay[pos..];
    let s: String = tail.iter().take(max_len)
        .take_while(|c| c.is_ascii_graphic() || **c == b' ' || **c == b'.')
        .map(|c| *c as char).collect();
    if s.len() >= prefix.len() + 2 { Some(s) } else { None }
}

fn read_alnum(hay: &[u8], max_len: usize) -> String {
    hay.iter().take(max_len)
        .take_while(|c| c.is_ascii_alphanumeric() || **c == b'-')
        .map(|c| *c as char).collect()
}

fn read_uuid(hay: &[u8]) -> Option<String> {
    let s: String = hay.iter()
        .take_while(|c| c.is_ascii_graphic() || **c == b'\n' || **c == b' ')
        .map(|c| *c as char).collect();
    let trimmed = s.split('\n').next()?.trim().to_string();
    if trimmed.len() >= 16 && trimmed.len() <= 64 && trimmed.contains('-') {
        Some(trimmed)
    } else { None }
}
