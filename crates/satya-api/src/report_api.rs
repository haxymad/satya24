//! Builds the detailed forensic PDF for JUAN (HeimVision) cases.

use satya_image::vfs::{volumes, Volume};
use satya_image::{open_image, FsKind, ImageSource};
use satya_parsers::juan::{self, JuanScan, SlotState};
use satya_report::forensic::*;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ReportOptions {
    pub case_id: String,
    pub case_name: String,
    pub examiner: String,
    pub organization: String,
    pub notes: String,
    /// Evidence frames to embed (0 = none). Default 8.
    pub thumbnails: Option<usize>,
}

pub(crate) fn build_and_write(
    image: &str,
    scan: &JuanScan,
    opts: &ReportOptions,
    activity: &[serde_json::Value],
) -> Result<(PathBuf, String), String> {
    let report = build(image, scan, opts, activity)?;
    let dir = crate::juan_api::out_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let out = dir.join(format!("forensic_report_{}.pdf", chrono::Utc::now().format("%Y%m%d_%H%M%S")));
    generate(&report, &out).map_err(|e| e.to_string())?;
    let bytes = std::fs::read(&out).map_err(|e| e.to_string())?;
    Ok((out, hex::encode(Sha256::digest(&bytes))))
}

fn build(image: &str, scan: &JuanScan, opts: &ReportOptions, activity: &[serde_json::Value]) -> Result<ForensicReport, String> {
    let src = open_image(Path::new(image)).map_err(|e| format!("open image: {e}"))?;
    let fp = juan::fingerprint(&scan.layout);
    let s = &scan.summary;
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S UTC").to_string();
    let ev_name = Path::new(image).file_stem().map(|x| x.to_string_lossy().into_owned()).unwrap_or_default();

    // --- evidence ----------------------------------------------------------
    let stored = src.stored_hashes();
    let acq = src.acquisition();
    let mut acquisition = Vec::new();
    let mut segments = Vec::new();
    let mut read_errors = 0;
    if let Some(a) = &acq {
        for (k, v) in [
            ("Imaging software", &a.software),
            ("Acquired on (tool's clock)", &a.acquired),
            ("Acquisition OS", &a.os),
            ("Case number (in E01)", &a.case_number),
            ("Evidence number (in E01)", &a.evidence_number),
            ("Description (in E01)", &a.description),
            ("Examiner (in E01)", &a.examiner),
            ("Notes (in E01)", &a.notes),
        ] {
            if let Some(v) = v {
                acquisition.push((k.to_string(), v.clone()));
            }
        }
        read_errors = a.read_errors.len();
        for seg in &a.segments {
            segments.push((seg.path.clone(), seg.bytes, file_sha256(&seg.path).unwrap_or_else(|e| format!("unreadable: {e}"))));
        }
    }
    let verified = crate::case_api::verification_for(image).map(|h| Verification {
        md5: h.md5,
        sha1: h.sha1,
        sha256: h.sha256,
        md5_matches: h.md5_matches,
        sha1_matches: h.sha1_matches,
    });

    // --- partitions --------------------------------------------------------
    let partitions = volumes(src.as_ref())
        .into_iter()
        .map(|p| {
            let role = if p.start_lba == scan.layout.fat_partition.start_lba {
                "video"
            } else if matches!(p.fs, FsKind::Ext2 | FsKind::Ext3 | FsKind::Ext4) {
                "system"
            } else {
                "other"
            };
            let detail = match Volume::open(src.as_ref(), &p) {
                Ok(v) => {
                    let i = v.info();
                    let label = if i.label.trim().is_empty() { String::new() } else { format!("label \"{}\", ", i.label.trim()) };
                    format!("{label}{}-byte units", i.cluster_or_block_size)
                }
                Err(e) => e.to_string(),
            };
            PartitionRow { index: p.index, fs: p.fs.as_str().into(), start_lba: p.start_lba, bytes: p.size(), role: role.into(), detail }
        })
        .collect();

    // --- recordings --------------------------------------------------------
    let mut recorded: Vec<&juan::SlotRecord> =
        scan.slots.iter().filter(|r| r.state == SlotState::Recorded && r.payload_offset.is_some()).collect();
    recorded.sort_by_key(|r| (r.header_start.unwrap_or(u32::MAX), r.slot));
    let fmt = |t: Option<chrono::DateTime<chrono::Utc>>| t.map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string()).unwrap_or_else(|| "-".into());
    let mut header_len: Vec<(u64, usize)> = s.header_len_hist.iter().map(|(k, v)| (*k, *v)).collect();
    header_len.sort_by(|a, b| b.1.cmp(&a.1));
    let mut fat_delta: Vec<(i64, usize)> = s.fat_delta_hist.iter().map(|(k, v)| (*k, *v)).collect();
    fat_delta.sort_by(|a, b| b.1.cmp(&a.1));
    let recording = Recording {
        first_utc: fmt(s.first_start_utc),
        last_utc: fmt(s.last_end_utc),
        recorded_seconds: s.recorded_seconds,
        slots_total: s.total_slots,
        recorded: s.recorded,
        empty: s.empty,
        residual: s.residual,
        unrecognized: s.unrecognized,
        slot_bytes: scan.layout.slot_size.unwrap_or(0),
        payload_bytes: s.payload_bytes,
        monotonic: s.slot_order_monotonic,
        overlaps: s.overlaps,
        gaps: s.gaps.clone(),
        est_fps: s.est_fps,
        distinct_sps: s.distinct_sps,
        nal_total: s.nal_total,
        nal_nonstandard: s.nal_nonstandard,
        pictures: s.pictures,
        header_len,
        fat_delta,
        hourly: hourly(&recorded),
    };
    let slots = recorded
        .iter()
        .map(|r| SlotRow {
            slot: r.slot,
            path: r.path.clone(),
            start: fmt(r.header_start_utc),
            end: fmt(r.header_end_utc),
            seconds: match (r.header_start, r.header_end) {
                (Some(a), Some(b)) => b as i64 - a as i64,
                _ => 0,
            },
            bytes: r.payload_len,
            sector: r.payload_extents.first().map(|x| x.disk_offset / 512).unwrap_or(0),
        })
        .collect();

    // --- deleted entries ---------------------------------------------------
    let deleted: Vec<DeletedRow> = crate::explorer_api::deleted_entries(image, 3)
        .into_iter()
        .map(|d| DeletedRow {
            location: format!("p{} {}", d["partition"], d["fs"].as_str().unwrap_or("")),
            path: d["path"].as_str().unwrap_or("").into(),
            id: d["id"].as_str().unwrap_or("").into(),
            size: d["size"].as_u64().unwrap_or(0),
            deleted_at: d["deleted_at"].as_str().unwrap_or("-").into(),
            status: if d["recoverable"].as_bool() == Some(true) { "readable".into() } else { "name/times only".into() },
        })
        .collect();

    // --- exports and activity (from the signed session log) ----------------
    let mut exports = Vec::new();
    let mut activity_rows = Vec::new();
    for a in activity {
        let ev = &a["event"];
        let action = ev["action"].as_str().unwrap_or("").to_string();
        if matches!(action.as_str(), "export" | "extract") {
            exports.push(ExportRow {
                file: ev["file"].as_str().or(ev["output"].as_str()).map(short_path).unwrap_or_default(),
                bytes: ev["bytes"].as_u64().unwrap_or(0),
                md5: ev["md5"].as_str().unwrap_or("-").into(),
                sha256: ev["sha256"].as_str().unwrap_or("-").into(),
            });
        }
        let detail = ev
            .as_object()
            .map(|o| {
                o.iter()
                    .filter(|(k, _)| k.as_str() != "action")
                    .map(|(k, v)| format!("{k}={}", v.as_str().map(short_path).unwrap_or_else(|| v.to_string())))
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_default();
        activity_rows.push(ActivityRow {
            time: a["ts"].as_str().unwrap_or("").chars().take(19).collect::<String>().replace('T', " "),
            action,
            detail,
            hash: a["hash"].as_str().unwrap_or("").chars().take(16).collect(),
        });
    }

    // --- findings ----------------------------------------------------------
    let mut findings = vec![format!(
        "The image is a {} disk from a {} recorder (identification confidence {:.0}%).",
        human_bytes(src.len()),
        fp.model.clone().unwrap_or_default(),
        fp.confidence * 100.0
    )];
    if s.recorded > 0 {
        findings.push(format!(
            "{} recording slots hold video from {} to {} (recorder clock, UTC), {} in total.",
            s.recorded,
            fmt(s.first_start_utc),
            fmt(s.last_end_utc),
            human_secs(s.recorded_seconds)
        ));
        findings.push(if s.gaps.is_empty() && s.overlaps == 0 {
            "Recording is continuous: no gap longer than 5 seconds and no overlap between consecutive slots.".to_string()
        } else {
            format!("{} gap(s) longer than 5 s and {} overlapping slot(s) were found (Section 5).", s.gaps.len(), s.overlaps)
        });
    }
    findings.push(format!(
        "{} of {} pre-allocated slots were never used; {} hold data without a header (recovery candidates).",
        s.empty, s.total_slots, s.residual
    ));
    if !deleted.is_empty() {
        findings.push(format!(
            "{} deleted filesystem entr{} found (Section 8).",
            deleted.len(),
            if deleted.len() == 1 { "y was" } else { "ies were" }
        ));
    }
    findings.push(match &verified {
        Some(v) if v.md5_matches == Some(true) => "The full media was re-hashed and matches the acquisition MD5.".to_string(),
        Some(v) if v.md5_matches == Some(false) => "WARNING: the re-computed MD5 does NOT match the acquisition MD5.".to_string(),
        Some(_) => "The full media was re-hashed (no stored hash to compare).".to_string(),
        None => "The image was not re-hashed in this session; integrity relies on the stored acquisition hash.".to_string(),
    });

    let limitations: Vec<String> = vec![
        "Times come from the recorder's own clock; no external anchor (NTP log, visible clock, known event) was available to confirm it.".into(),
        "FAT32 directory times are local time without a timezone; they are compared to slot headers, not converted.".into(),
        "Deleted ext3 files usually keep their names and times but not their contents (the block map is zeroed on deletion).".into(),
        "Unused slots were checked by sampling 16 points each; data between sample points would be missed unless a deep scan is run.".into(),
        "Frames per second and MP4 timing are estimates for playback; they are not evidence of when frames were recorded.".into(),
    ];

    let mut warnings: Vec<String> = crate::juan_api::warnings_for(scan);
    if read_errors > 0 {
        warnings.push(format!("The imaging tool recorded {read_errors} unreadable sector range(s)."));
    }

    let thumbnails = thumbnails(src.as_ref(), &recorded, opts.thumbnails.unwrap_or(8).min(24));

    Ok(ForensicReport {
        case_id: or(&opts.case_id, &ev_name),
        case_name: or(&opts.case_name, &format!("Examination of {ev_name}")),
        examiner: opts.examiner.clone(),
        organization: opts.organization.clone(),
        notes: opts.notes.clone(),
        generated_utc: now,
        tool: format!("SATYA {} (satya-api)", env!("CARGO_PKG_VERSION")),
        evidence: Evidence {
            path: image.into(),
            container: src.kind().into(),
            media_bytes: src.len(),
            stored_md5: stored.md5,
            stored_sha1: stored.sha1,
            verified,
            acquisition,
            read_errors,
            segments,
        },
        device: DeviceView {
            family: "JUAN NVR".into(),
            model: fp.model.unwrap_or_default(),
            manufacturer: "JUAN (OEM for HeimVision / Zosi / Hiseeu / Kogan)".into(),
            confidence: fp.confidence,
            signals: scan.layout.signals.clone(),
        },
        disk_bytes: src.len(),
        partitions,
        recording,
        thumbnails,
        slots,
        deleted,
        exports,
        activity: activity_rows,
        session_public_key: crate::case_api::session_public_key(),
        findings,
        warnings,
        limitations,
    })
}

/// Minutes recorded per hour of the header clock (UTC), over the recorded span.
fn hourly(recorded: &[&juan::SlotRecord]) -> Vec<(String, f64)> {
    let mut buckets: BTreeMap<i64, f64> = BTreeMap::new();
    for r in recorded {
        let (Some(a), Some(b)) = (r.header_start, r.header_end) else { continue };
        let (mut t, end) = (a as i64, b as i64);
        while t < end {
            let hour = t / 3600;
            let next = ((hour + 1) * 3600).min(end);
            *buckets.entry(hour).or_default() += (next - t) as f64 / 60.0;
            t = next;
        }
    }
    let (Some(&first), Some(&last)) = (buckets.keys().next(), buckets.keys().last()) else { return Vec::new() };
    if last - first > 24 * 14 {
        return Vec::new(); // more than two weeks of bars would be unreadable
    }
    (first..=last)
        .map(|h| {
            let label = chrono::DateTime::from_timestamp(h * 3600, 0).map(|d| d.format("%d %Hh").to_string()).unwrap_or_default();
            (label, buckets.get(&h).copied().unwrap_or(0.0).min(60.0))
        })
        .collect()
}

/// Decode the first picture of `n` slots spread evenly over the recording.
fn thumbnails(src: &dyn ImageSource, recorded: &[&juan::SlotRecord], n: usize) -> Vec<Thumbnail> {
    if n == 0 || recorded.is_empty() {
        return Vec::new();
    }
    let dir = crate::juan_api::out_dir().join("report_assets");
    if std::fs::create_dir_all(&dir).is_err() {
        return Vec::new();
    }
    let picks: Vec<usize> = if recorded.len() <= n {
        (0..recorded.len()).collect()
    } else {
        (0..n).map(|i| i * (recorded.len() - 1) / (n - 1).max(1)).collect()
    };
    let mut out = Vec::new();
    for i in picks {
        let r = recorded[i];
        // The first ~4 MiB of payload always contains the slot's first key frame.
        let want = r.payload_len.min(4 << 20);
        let mut data: Vec<u8> = Vec::with_capacity(want as usize);
        for x in &r.payload_extents {
            if data.len() as u64 >= want {
                break;
            }
            let take = x.len.min(want - data.len() as u64) as usize;
            match src.read_vec(x.disk_offset, take) {
                Ok(b) => data.extend_from_slice(&b),
                Err(_) => break,
            }
        }
        let hevc = dir.join(format!("slot{}.hevc", r.slot));
        let jpg = dir.join(format!("slot{}.jpg", r.slot));
        if std::fs::write(&hevc, juan::clean_hevc(&data)).is_err() {
            continue;
        }
        let ok = std::process::Command::new("ffmpeg")
            .args(["-y", "-hide_banner", "-loglevel", "error", "-f", "hevc", "-i"])
            .arg(&hevc)
            .args(["-frames:v", "1", "-vf", "scale=640:-2", "-q:v", "3"])
            .arg(&jpg)
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        let _ = std::fs::remove_file(&hevc);
        let jpeg = if ok { std::fs::read(&jpg).unwrap_or_default() } else { Vec::new() };
        let start = r.header_start_utc.map(|t| t.format("%Y-%m-%d %H:%M:%S UTC").to_string()).unwrap_or_else(|| "-".into());
        out.push(Thumbnail {
            jpeg,
            caption: format!(
                "Slot {} - {} - {} - disk sector {}",
                r.slot,
                r.path,
                start,
                r.payload_extents.first().map(|x| x.disk_offset / 512).unwrap_or(0)
            ),
        });
    }
    out
}

fn file_sha256(path: &str) -> Result<String, String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 8 << 20];
    loop {
        let n = f.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(hex::encode(h.finalize()))
}

fn short_path(p: &str) -> String {
    Path::new(p).file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| p.to_string())
}

fn or(v: &str, default: &str) -> String {
    if v.trim().is_empty() { default.to_string() } else { v.trim().to_string() }
}
