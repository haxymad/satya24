//! Old-UI support for JUAN (HeimVision/Zosi) images, E01 or raw.
//!
//! `/api/analyze` calls [`analyze`] when it detects a JUAN layout. It runs
//! the streaming slot scan and returns the same data the legacy analyzer
//! produces (frames, timeline, summary), so every existing tab works
//! unchanged. Nothing reads the whole image into RAM.
//!
//! In this mode one "frame" in the UI is one recording slot (one
//! `dirNNNNN/fileNNNN.dat`, about 107 s of video on the CFReDS unit).

use crate::juan_api::{self, ExportRequest};
use satya_core::analysis::{AllocationRegion, AnalysisSummary, Channel, DeviceInfo, Partition, VideoStats};
use satya_core::{Codec, DeviceFingerprint, RecoveredFrame, RecoverySource};
use satya_parsers::juan::{self, ExportOptions, JuanScan, JuanVolume, ScanOptions, SlotState};
use satya_trust::{fuse_with_diagnostics, to_claim, FusedTimestamp, Source};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Slots in the quick viewing copy made during Analyze (about 107 s each).
const DEFAULT_PREVIEW_SLOTS: usize = 3;

pub struct JuanCase {
    pub fp: DeviceFingerprint,
    pub frames: Vec<RecoveredFrame>,
    pub timeline: Vec<FusedTimestamp>,
    pub summary: AnalysisSummary,
    pub scan: Arc<JuanScan>,
    pub size_bytes: u64,
    /// MD5 recorded in the E01 at acquisition (not recomputed here).
    pub md5: String,
    /// Not computed during Analyze: hashing 150 GB takes too long. Use
    /// `satya-cli verify` for a full SHA-256 and stored-hash check.
    pub sha256: String,
    pub preview_mp4: Option<PathBuf>,
}

pub fn analyze(path: &str) -> Result<JuanCase, String> {
    let src = satya_image::open_image(Path::new(path)).map_err(|e| format!("open image: {e}"))?;
    let vol = JuanVolume::open(src.as_ref())
        .map_err(|e| format!("open volume: {e}"))?
        .ok_or("no JUAN layout found")?;
    let scan = vol
        .scan(&ScanOptions::default(), |_, _| {})
        .map_err(|e| format!("scan: {e}"))?;

    let fp = juan::fingerprint(&scan.layout);
    let frames = ordered_frames(&scan);
    // Every frame with a header time gets a timeline entry, and those frames
    // come first, so timeline[i] always belongs to frames[i].
    let timeline: Vec<FusedTimestamp> = frames
        .iter()
        .take_while(|f| !f.claims.is_empty())
        .map(|f| {
            let c = &f.claims[0];
            fuse_with_diagnostics(&[to_claim(c.claimed_utc.timestamp() as f64, Source::FrameHeader, c.confidence as f64)])
        })
        .collect();

    let stored = src.stored_hashes();
    let summary = build_summary(src.as_ref(), &scan, &frames, &fp);
    let preview_mp4 = make_preview(&vol, &scan);

    Ok(JuanCase {
        fp,
        frames,
        timeline,
        summary,
        scan: Arc::new(scan),
        size_bytes: src.len(),
        md5: stored.md5.unwrap_or_else(|| "none stored".into()),
        sha256: "not computed (run satya-cli verify)".into(),
        preview_mp4,
    })
}

/// Recorded slots in time order first, then header-less (recovered) slots.
fn ordered_frames(scan: &JuanScan) -> Vec<RecoveredFrame> {
    let mut frames = juan::to_recovered_frames(scan);
    frames.sort_by_key(|f| match f.claims.first() {
        Some(c) => (0u8, c.claimed_utc.timestamp(), f.offset),
        None => (1u8, 0, f.offset),
    });
    frames
}

fn build_summary(
    src: &dyn satya_image::ImageSource,
    scan: &JuanScan,
    frames: &[RecoveredFrame],
    fp: &DeviceFingerprint,
) -> AnalysisSummary {
    let s = &scan.summary;
    let device = DeviceInfo {
        oem: fp.oem.as_str().into(),
        model: fp.model.clone(),
        firmware: None,
        serial: None,
        mac: None,
        uuid: None,
        block_size: scan.layout.slot_size.map(|v| v as u32),
        sector_size: Some(512),
        total_bytes: src.len(),
        disk_size_gb: src.len() as f64 / 1e9,
        channels: (s.distinct_sps > 0).then_some(s.distinct_sps as u32),
        manufacturer_string: Some("JUAN (OEM for HeimVision / Zosi / Hiseeu / Kogan)".into()),
    };

    let partitions = satya_image::read_partitions(src)
        .unwrap_or_default()
        .into_iter()
        .map(|p| {
            let role = if p.start_lba == scan.layout.fat_partition.start_lba {
                "video"
            } else if matches!(p.fs, satya_image::FsKind::Ext2 | satya_image::FsKind::Ext3 | satya_image::FsKind::Ext4) {
                "system"
            } else {
                "other"
            };
            Partition {
                index: p.index,
                name: if p.name.is_empty() { format!("partition {}", p.index) } else { p.name.clone() },
                offset_bytes: p.offset(),
                size_bytes: p.size(),
                fs_type: p.fs.as_str().into(),
                role: role.into(),
            }
        })
        .collect();

    // A "channel" here is a distinct H.265 stream (by SPS). Several SPS can
    // mean several cameras or several profiles of one camera.
    let mut by_sps: BTreeMap<String, Vec<&satya_parsers::juan::SlotRecord>> = BTreeMap::new();
    for r in scan.slots.iter().filter(|r| r.state == SlotState::Recorded && r.payload_offset.is_some()) {
        let key = r.sps_ids.first().cloned().unwrap_or_else(|| "no-sps".into());
        by_sps.entry(key).or_default().push(r);
    }
    let channels = by_sps
        .into_iter()
        .enumerate()
        .map(|(i, (sps, slots))| {
            let offs: Vec<u64> = slots.iter().filter_map(|r| r.payload_extents.first().map(|x| x.disk_offset)).collect();
            Channel {
                id: i as u32,
                label: format!("Stream {} (SPS {})", i + 1, &sps[..sps.len().min(8)]),
                frame_count: slots.len(),
                total_bytes: slots.iter().map(|r| r.payload_len).sum(),
                first_offset: offs.iter().copied().min().unwrap_or(0),
                last_offset: offs.iter().copied().max().unwrap_or(0),
                resolution: None,
                codec: "H265".into(),
            }
        })
        .collect();

    let mut allocation: Vec<AllocationRegion> = scan
        .slots
        .iter()
        .filter(|r| matches!(r.state, SlotState::Recorded | SlotState::Residual))
        .flat_map(|r| {
            let kind = if r.state == SlotState::Recorded { "allocated" } else { "residual" };
            r.payload_extents.iter().map(move |x| AllocationRegion {
                start: x.disk_offset,
                end: x.disk_offset + x.len,
                kind: kind.into(),
                frame_count: 1,
            })
        })
        .collect();
    allocation.sort_by_key(|a| a.start);

    let allocated = frames.iter().filter(|f| f.recovery_source == RecoverySource::Allocated).count();
    let seconds = s.recorded_seconds as f64;
    let stats = VideoStats {
        total_frames: frames.len(),
        allocated,
        carved: frames.len() - allocated,
        partial: 0,
        h264: frames.iter().filter(|f| f.codec == Codec::H264).count(),
        h265: frames.iter().filter(|f| f.codec == Codec::H265).count(),
        key_frames: scan.slots.iter().map(|r| r.nal.idr as usize).sum(),
        total_video_bytes: s.payload_bytes,
        bitrate_kbps: (seconds > 0.0).then(|| s.payload_bytes as f64 * 8.0 / seconds / 1000.0),
        duration_seconds: (seconds > 0.0).then_some(seconds),
        fps_estimate: s.est_fps,
    };

    AnalysisSummary { device, partitions, channels, logs: Vec::new(), allocation, stats }
}

/// Short H.264 viewing copy of the first recorded slots, so the Viewer and
/// ML tabs work right after Analyze. Browsers often cannot play H.265; this
/// copy is re-encoded and is for viewing only, never evidence.
fn make_preview(vol: &JuanVolume, scan: &JuanScan) -> Option<PathBuf> {
    let n = std::env::var("SATYA_PREVIEW_SLOTS").ok().and_then(|v| v.parse().ok()).unwrap_or(DEFAULT_PREVIEW_SLOTS);
    let mut recorded: Vec<_> = scan
        .slots
        .iter()
        .filter(|r| r.state == SlotState::Recorded && r.payload_offset.is_some())
        .collect();
    recorded.sort_by_key(|r| (r.header_start.unwrap_or(u32::MAX), r.slot));
    let chosen: Vec<_> = recorded.into_iter().take(n.max(1)).cloned().collect();
    if chosen.is_empty() {
        return None;
    }
    let subset = JuanScan { layout: scan.layout.clone(), summary: scan.summary.clone(), slots: chosen };

    let dir = abs_out_dir();
    std::fs::create_dir_all(&dir).ok()?;
    let hevc = dir.join("preview.hevc");
    let mp4 = dir.join("preview.mp4");
    let opts = ExportOptions { strip_nonstandard: true, slot_range: None, include_residual: false };
    let mut f = std::io::BufWriter::new(std::fs::File::create(&hevc).ok()?);
    let report = vol.export_hevc(&subset, &mut f, &opts).ok()?;
    drop(f);
    let fps = report.est_fps.filter(|v| v.is_finite() && *v >= 1.0 && *v <= 120.0).unwrap_or(25.0);

    let ok = satya_video::hevc_to_h264_preview(&hevc, &mp4, fps)
        .or_else(|e| {
            tracing::warn!("H.264 preview failed ({e}); falling back to H.265 in MP4");
            satya_video::hevc_to_mp4(&hevc, &mp4, fps)
        })
        .is_ok();
    let _ = std::fs::remove_file(&hevc);
    ok.then_some(mp4)
}

/// Full evidence export for the "Export MP4" button: every recorded slot as
/// H.265 (non-standard units stripped), an MP4 wrapper (no re-encoding) and
/// a provenance manifest with hashes, all in the output folder.
pub fn export_all(image_path: &str, scan: &JuanScan) -> Result<juan_api::ExportResult, String> {
    let req = ExportRequest { from: None, to: None, strip: true, residual: false, fps: None };
    juan_api::run_export(image_path, scan, &req)
}

pub fn abs_out_dir() -> PathBuf {
    juan_api::out_dir()
}

/// Read `len` bytes at `offset` of the logical media (raw or E01), clamped
/// to the end of the media. Used by the hex views.
pub fn read_range(path: &str, offset: u64, len: usize) -> Result<(u64, Vec<u8>), String> {
    let src = satya_image::open_image(Path::new(path)).map_err(|e| format!("open image: {e}"))?;
    let start = offset.min(src.len());
    let n = (len as u64).min(src.len() - start) as usize;
    let data = src.read_vec(start, n).map_err(|e| format!("read: {e}"))?;
    Ok((start, data))
}

/// Whole logical media as bytes, for the legacy in-memory parsers.
/// Works for raw and E01; callers must check the size first.
pub fn read_all(path: &str) -> Result<Vec<u8>, String> {
    let src = satya_image::open_image(Path::new(path)).map_err(|e| format!("open image: {e}"))?;
    src.read_vec(0, src.len() as usize).map_err(|e| format!("read: {e}"))
}
