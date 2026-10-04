//! JUAN NVR family (sold as HeimVision, Zosi, Hiseeu, Kogan K9604-W and siblings).
//!
//! Layout, as observed on the NIST CFReDS "Heimvision DVR" image:
//! - GPT disk: an ext3 system partition and a FAT32 video partition.
//! - FAT32 root holds `ident.bin`, `index.bin` and `dir00000`..`dirNNNNN`.
//! - Each `dirNNNNN` holds pre-allocated, fixed-size slots `file0000.dat`...
//!   (8 MiB on the CFReDS unit), reused as a ring buffer.
//! - A recorded slot starts with the magic `"luo "`, then u32 LE start and end
//!   times (Unix epoch, device clock), a proprietary index region, then an
//!   Annex-B H.265 elementary stream.
//!
//! Claims that are still being verified are measured rather than assumed:
//! header length, whether slots interleave several cameras, which NAL units
//! are non-standard, and how the header clock relates to FAT directory times.
//! `scan` reports all of these so the format notes can be checked on real media.

use satya_core::*;
use chrono::{DateTime, NaiveDateTime, Utc};
use md5::Md5;
use memchr::memmem;
use satya_image::fat32::{Extent, Fat32, FatEntry, FileMap};
use satya_image::{probe_fs, read_partitions, FsKind, ImageSource, Partition};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::io::{self, Write};

pub const MAGIC: &[u8; 4] = b"luo ";
const HEAD_PROBE: usize = 64 * 1024;
/// Bytes covered by the known header fields (magic + two u32 times).
const HEADER_FIELDS: usize = 12;
const SAMPLE_POINTS: u64 = 16;
const SAMPLE_LEN: usize = 4096;
const GAP_REPORT_S: i64 = 5;

#[derive(Debug, Clone, Serialize)]
pub struct JuanLayout {
    pub fat_partition: Partition,
    pub ext_partition: Option<Partition>,
    pub confidence: f32,
    pub signals: Vec<String>,
    pub slot_dirs: usize,
    pub slots: usize,
    pub slot_size: Option<u64>,
    pub cluster_size: u64,
    pub deleted_dir_entries: usize,
}

struct SlotRef {
    dir: u32,
    file: u32,
    entry: FatEntry,
}

/// An opened JUAN video volume.
pub struct JuanVolume<'a> {
    src: &'a dyn ImageSource,
    fat: Fat32<'a>,
    pub layout: JuanLayout,
    slots: Vec<SlotRef>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum SlotState {
    /// Starts with the `luo ` header.
    Recorded,
    /// All sampled bytes are zero (never used, or wiped).
    Empty,
    /// No header, but non-zero data: recovery candidate.
    Residual,
    /// Non-zero first bytes that are not a `luo ` header.
    Unrecognized,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct NalStats {
    pub total: u64,
    pub vps: u64,
    pub sps: u64,
    pub pps: u64,
    pub idr: u64,
    pub cra: u64,
    pub pictures: u64,
    /// Start codes whose next two bytes are not a valid H.265 NAL header.
    pub nonstandard: u64,
    /// H.265 nal_unit_type -> count.
    pub types: BTreeMap<u8, u64>,
    /// First header byte of non-standard units (hex) -> count.
    pub nonstandard_first_byte: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SlotRecord {
    pub slot: u32,
    pub dir: u32,
    pub file: u32,
    pub path: String,
    pub state: SlotState,
    pub header_start: Option<u32>,
    pub header_end: Option<u32>,
    pub header_start_utc: Option<DateTime<Utc>>,
    pub header_end_utc: Option<DateTime<Utc>>,
    pub fat_created: Option<NaiveDateTime>,
    pub fat_modified: Option<NaiveDateTime>,
    /// FAT created time minus header end time, in seconds (both device clock).
    pub fat_minus_header_end_s: Option<i64>,
    /// Offset of the first H.265 start code inside the slot (= header length).
    pub payload_offset: Option<u64>,
    pub payload_len: u64,
    /// Absolute disk ranges holding the payload (provenance).
    pub payload_extents: Vec<Extent>,
    pub slot_contiguous: bool,
    pub nal: NalStats,
    pub sps_ids: Vec<String>,
    pub payload_sha256: Option<String>,
    pub first_bytes_hex: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct DirSummary {
    pub dir: u32,
    pub slots: usize,
    pub recorded: usize,
    pub first_start_utc: Option<DateTime<Utc>>,
    pub last_end_utc: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ScanSummary {
    pub total_slots: usize,
    pub recorded: usize,
    pub empty: usize,
    pub residual: usize,
    pub unrecognized: usize,
    pub fat_dated_slots: usize,
    pub first_start_utc: Option<DateTime<Utc>>,
    pub last_end_utc: Option<DateTime<Utc>>,
    /// True if header start times never go backwards in slot order.
    pub slot_order_monotonic: bool,
    /// Recorded slots whose start is before the previous slot's end.
    pub overlaps: usize,
    /// (slot, seconds) where the gap to the previous slot exceeds 5 s.
    pub gaps: Vec<(u32, i64)>,
    pub header_len_hist: BTreeMap<u64, usize>,
    pub fat_delta_hist: BTreeMap<i64, usize>,
    pub distinct_sps: usize,
    pub nal_total: u64,
    pub nal_nonstandard: u64,
    pub pictures: u64,
    pub recorded_seconds: i64,
    /// pictures / recorded seconds (all streams combined).
    pub est_fps: Option<f64>,
    pub payload_bytes: u64,
    pub dirs: Vec<DirSummary>,
}

#[derive(Debug, Clone, Serialize)]
pub struct JuanScan {
    pub layout: JuanLayout,
    pub summary: ScanSummary,
    pub slots: Vec<SlotRecord>,
}

#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    /// Read every byte of header-less slots instead of sampling 16 points.
    pub deep: bool,
    /// Hash each recorded payload (SHA-256).
    pub hash_payloads: bool,
}

#[derive(Debug, Clone, Default)]
pub struct ExportOptions {
    /// Keep only NAL units with a valid H.265 header.
    pub strip_nonstandard: bool,
    /// Inclusive slot-number range.
    pub slot_range: Option<(u32, u32)>,
    /// Include Residual slots (no header) after the recorded ones.
    pub include_residual: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExportReport {
    pub slots_written: usize,
    pub slot_order: Vec<u32>,
    pub bytes_written: u64,
    pub nal_kept: u64,
    pub nal_dropped: u64,
    pub md5: String,
    pub sha256: String,
    pub est_fps: Option<f64>,
}

impl<'a> JuanVolume<'a> {
    /// Detect and open a JUAN layout. Returns Ok(None) if the image is not one.
    pub fn open(src: &'a dyn ImageSource) -> io::Result<Option<Self>> {
        let mut parts = read_partitions(src).unwrap_or_default();
        if parts.is_empty() && probe_fs(src, 0) == FsKind::Fat32 {
            parts.push(Partition {
                index: 0,
                scheme: "none",
                start_lba: 0,
                end_lba: src.len() / 512 - 1,
                type_id: String::new(),
                name: "whole image".into(),
                fs: FsKind::Fat32,
            });
        }
        let ext = parts.iter().find(|p| matches!(p.fs, FsKind::Ext2 | FsKind::Ext3)).cloned();

        for part in parts.iter().filter(|p| p.fs == FsKind::Fat32) {
            let Ok(fat) = Fat32::open(src, part.offset()) else { continue };
            let Ok(root) = fat.list_root() else { continue };
            let live = |n: &str| root.iter().find(|e| !e.deleted && e.name.eq_ignore_ascii_case(n));

            let mut signals = Vec::new();
            let mut conf = 0.0f32;
            if live("ident.bin").is_some() {
                conf += 0.25;
                signals.push("FAT32 root has ident.bin".to_string());
            }
            if live("index.bin").is_some() {
                conf += 0.25;
                signals.push("FAT32 root has index.bin".to_string());
            }
            let mut dirs: Vec<(u32, FatEntry)> = root
                .iter()
                .filter(|e| e.is_dir && !e.deleted)
                .filter_map(|e| numbered(&e.name, "dir", "").map(|n| (n, e.clone())))
                .collect();
            dirs.sort_by_key(|d| d.0);
            if dirs.first().map(|d| d.0) == Some(0) {
                conf += 0.2;
                signals.push(format!("{} dirNNNNN slot directories", dirs.len()));
            }
            if conf < 0.45 {
                continue;
            }

            let mut slots = Vec::new();
            let mut deleted = root.iter().filter(|e| e.deleted).count();
            for (dn, dir) in &dirs {
                let Ok(entries) = fat.list_dir(dir) else { continue };
                deleted += entries.iter().filter(|e| e.deleted).count();
                let mut files: Vec<(u32, FatEntry)> = entries
                    .into_iter()
                    .filter(|e| !e.is_dir && !e.deleted)
                    .filter_map(|e| numbered(&e.name, "file", ".dat").map(|n| (n, e)))
                    .collect();
                files.sort_by_key(|f| f.0);
                slots.extend(files.into_iter().map(|(fnum, entry)| SlotRef { dir: *dn, file: fnum, entry }));
            }
            if slots.is_empty() {
                continue;
            }
            conf += 0.1;
            signals.push(format!("{} fileNNNN.dat slots", slots.len()));

            let sizes: BTreeSet<u32> = slots.iter().map(|s| s.entry.size).collect();
            let slot_size = (sizes.len() == 1).then(|| *sizes.iter().next().unwrap() as u64);
            if let Some(sz) = slot_size {
                signals.push(format!("all slots pre-allocated at {sz} bytes"));
            }
            // First non-empty slot should carry the magic.
            for s in slots.iter().take(64) {
                let map = fat.file_map(&s.entry);
                let mut head = [0u8; 4];
                if fat.read_at(&map, 0, &mut head).unwrap_or(0) == 4 && head != [0; 4] {
                    if &head == MAGIC {
                        conf += 0.15;
                        signals.push("slot header magic 'luo '".to_string());
                    }
                    break;
                }
            }
            if ext.is_some() {
                conf += 0.05;
                signals.push("ext2/3 system partition present".to_string());
            }
            if conf < 0.6 {
                continue;
            }
            let layout = JuanLayout {
                fat_partition: part.clone(),
                ext_partition: ext.clone(),
                confidence: conf.min(1.0),
                signals,
                slot_dirs: dirs.len(),
                slots: slots.len(),
                slot_size,
                cluster_size: fat.cluster_size,
                deleted_dir_entries: deleted,
            };
            return Ok(Some(Self { src, fat, layout, slots }));
        }
        Ok(None)
    }

    /// Classify every slot and gather verification statistics.
    pub fn scan(&self, opts: &ScanOptions, mut progress: impl FnMut(usize, usize)) -> io::Result<JuanScan> {
        let mut records = Vec::with_capacity(self.slots.len());
        let mut all_sps = BTreeSet::new();
        for (i, s) in self.slots.iter().enumerate() {
            if i % 256 == 0 {
                progress(i, self.slots.len());
            }
            let rec = self.scan_slot(i as u32, s, opts)?;
            all_sps.extend(rec.sps_ids.iter().cloned());
            records.push(rec);
        }
        progress(self.slots.len(), self.slots.len());
        let summary = summarize(&records, all_sps.len());
        Ok(JuanScan { layout: self.layout.clone(), summary, slots: records })
    }

    fn scan_slot(&self, idx: u32, s: &SlotRef, opts: &ScanOptions) -> io::Result<SlotRecord> {
        let map = self.fat.file_map(&s.entry);
        let size = s.entry.size as usize;
        let mut rec = SlotRecord {
            slot: idx,
            dir: s.dir,
            file: s.file,
            path: format!("dir{:05}/file{:04}.dat", s.dir, s.file),
            state: SlotState::Empty,
            header_start: None,
            header_end: None,
            header_start_utc: None,
            header_end_utc: None,
            fat_created: s.entry.created,
            fat_modified: s.entry.modified,
            fat_minus_header_end_s: None,
            payload_offset: None,
            payload_len: 0,
            payload_extents: Vec::new(),
            slot_contiguous: map.is_contiguous(),
            nal: NalStats::default(),
            sps_ids: Vec::new(),
            payload_sha256: None,
            first_bytes_hex: None,
        };

        let mut head = vec![0u8; HEAD_PROBE.min(size)];
        let n = self.fat.read_at(&map, 0, &mut head)?;
        head.truncate(n);

        let has_magic = head.len() >= 12 && &head[0..4] == MAGIC;
        // Only the fixed header fields decide "unrecognized": a slot whose
        // header was wiped but whose video survives is Residual, not unknown.
        let header_zero = head.iter().take(HEADER_FIELDS).all(|&b| b == 0);
        let head_zero = head.iter().all(|&b| b == 0);
        let needs_full = if has_magic {
            rec.state = SlotState::Recorded;
            true
        } else if !header_zero {
            rec.state = SlotState::Unrecognized;
            rec.first_bytes_hex = Some(hex::encode(&head[..head.len().min(16)]));
            true
        } else if !head_zero || opts.deep {
            true
        } else {
            !self.sampled_zero(&map, size as u64)?
        };
        if !needs_full {
            return Ok(rec);
        }

        let mut data = vec![0u8; size];
        let n = self.fat.read_at(&map, 0, &mut data)?;
        data.truncate(n);
        let Some(last) = data.iter().rposition(|&b| b != 0) else {
            rec.state = SlotState::Empty;
            return Ok(rec);
        };
        let end = last + 1;
        if rec.state == SlotState::Empty {
            rec.state = SlotState::Residual;
        }

        let search_from = if has_magic { 12 } else { 0 };
        if has_magic {
            let st = u32le(&data, 4);
            let en = u32le(&data, 8);
            rec.header_start = Some(st);
            rec.header_end = Some(en);
            rec.header_start_utc = DateTime::from_timestamp(st as i64, 0);
            rec.header_end_utc = DateTime::from_timestamp(en as i64, 0);
            if let (Some(c), Some(e)) = (s.entry.created, rec.header_end_utc) {
                rec.fat_minus_header_end_s = Some((c - e.naive_utc()).num_seconds());
            }
        }
        let Some(p0) = first_hevc_start(&data[..end], search_from) else {
            return Ok(rec);
        };
        let payload = &data[p0..end];
        rec.payload_offset = Some(p0 as u64);
        rec.payload_len = payload.len() as u64;
        rec.payload_extents = sub_extents(&map, p0 as u64, payload.len() as u64);
        let (stats, sps) = nal_stats(payload);
        rec.nal = stats;
        rec.sps_ids = sps.into_iter().collect();
        if opts.hash_payloads {
            rec.payload_sha256 = Some(hex::encode(Sha256::digest(payload)));
        }
        Ok(rec)
    }

    fn sampled_zero(&self, map: &FileMap, size: u64) -> io::Result<bool> {
        if size <= SAMPLE_LEN as u64 {
            return Ok(true);
        }
        let mut buf = vec![0u8; SAMPLE_LEN];
        for k in 1..=SAMPLE_POINTS {
            let off = (size - SAMPLE_LEN as u64) * k / SAMPLE_POINTS;
            let n = self.fat.read_at(map, off, &mut buf)?;
            if buf[..n].iter().any(|&b| b != 0) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Write recorded payloads as one Annex-B H.265 stream, ordered by header
    /// start time (then slot number), and hash what was written.
    pub fn export_hevc(&self, scan: &JuanScan, out: &mut dyn Write, opts: &ExportOptions) -> io::Result<ExportReport> {
        let in_range = |r: &SlotRecord| opts.slot_range.map_or(true, |(a, b)| r.slot >= a && r.slot <= b);
        let mut chosen: Vec<&SlotRecord> = scan
            .slots
            .iter()
            .filter(|r| r.payload_offset.is_some() && in_range(r))
            .filter(|r| r.state == SlotState::Recorded || (opts.include_residual && r.state == SlotState::Residual))
            .collect();
        chosen.sort_by_key(|r| (r.state != SlotState::Recorded, r.header_start.unwrap_or(u32::MAX), r.slot));

        let mut w = HashingWriter::new(out);
        let (mut kept, mut dropped, mut pictures, mut seconds) = (0u64, 0u64, 0u64, 0i64);
        for r in &chosen {
            let s = &self.slots[r.slot as usize];
            let map = self.fat.file_map(&s.entry);
            let mut payload = vec![0u8; r.payload_len as usize];
            let n = self.fat.read_at(&map, r.payload_offset.unwrap(), &mut payload)?;
            payload.truncate(n);
            if opts.strip_nonstandard {
                for (a, b) in nal_spans(&payload) {
                    if is_valid_hevc_header(&payload[a..b]) {
                        w.write_all(&[0, 0, 0, 1])?;
                        w.write_all(&payload[a..b])?;
                        kept += 1;
                    } else {
                        dropped += 1;
                    }
                }
            } else {
                w.write_all(&payload)?;
                kept += r.nal.total;
            }
            pictures += r.nal.pictures;
            if let (Some(a), Some(b)) = (r.header_start, r.header_end) {
                seconds += (b as i64 - a as i64).max(0);
            }
        }
        w.flush()?;
        let (bytes, md5, sha256) = w.finish();
        Ok(ExportReport {
            slots_written: chosen.len(),
            slot_order: chosen.iter().map(|r| r.slot).collect(),
            bytes_written: bytes,
            nal_kept: kept,
            nal_dropped: dropped,
            md5,
            sha256,
            est_fps: (seconds > 0).then(|| pictures as f64 / seconds as f64),
        })
    }

    pub fn source(&self) -> &dyn ImageSource {
        self.src
    }
}

/// Legacy entry point for in-memory images (`identify_device`).
pub struct JuanFs;

impl JuanFs {
    pub fn identify(image: &[u8]) -> Option<DeviceFingerprint> {
        let src = satya_image::MemImage(image);
        let vol = JuanVolume::open(&src).ok()??;
        Some(fingerprint(&vol.layout))
    }

    pub fn enumerate_frames(image: &[u8]) -> Result<Vec<RecoveredFrame>> {
        let src = satya_image::MemImage(image);
        let vol = JuanVolume::open(&src)?.ok_or_else(|| DvrError::Validation("not a JUAN volume".into()))?;
        let scan = vol.scan(&ScanOptions::default(), |_, _| {})?;
        Ok(to_recovered_frames(&scan))
    }
}

pub fn fingerprint(layout: &JuanLayout) -> DeviceFingerprint {
    DeviceFingerprint {
        oem: Oem::Juan,
        model: Some("JUAN NVR (HeimVision/Zosi K9604-W family)".into()),
        firmware: None,
        block_size: layout.slot_size.map(|s| s as u32),
        confidence: layout.confidence,
    }
}

/// One RecoveredFrame per slot payload, located at its first disk extent.
pub fn to_recovered_frames(scan: &JuanScan) -> Vec<RecoveredFrame> {
    scan.slots
        .iter()
        .filter(|r| matches!(r.state, SlotState::Recorded | SlotState::Residual))
        .filter_map(|r| {
            let first = r.payload_extents.first()?;
            let claims = r
                .header_start_utc
                .map(|t| {
                    vec![TimestampClaim {
                        frame_offset: first.disk_offset,
                        claimed_utc: t,
                        source: TimestampSource::FrameHeader,
                        confidence: 0.8,
                    }]
                })
                .unwrap_or_default();
            Some(RecoveredFrame {
                offset: first.disk_offset,
                length: r.payload_len.min(u32::MAX as u64) as u32,
                codec: Codec::H265,
                claims,
                recovery_source: if r.state == SlotState::Recorded {
                    RecoverySource::Allocated
                } else {
                    RecoverySource::Carved
                },
                recovery_confidence: if r.slot_contiguous { 0.95 } else { 0.7 },
            })
        })
        .collect()
}

fn summarize(records: &[SlotRecord], distinct_sps: usize) -> ScanSummary {
    let mut s = ScanSummary { total_slots: records.len(), distinct_sps, slot_order_monotonic: true, ..Default::default() };
    let mut prev: Option<&SlotRecord> = None;
    let mut dirs: BTreeMap<u32, DirSummary> = BTreeMap::new();
    for r in records {
        let d = dirs.entry(r.dir).or_insert(DirSummary {
            dir: r.dir,
            slots: 0,
            recorded: 0,
            first_start_utc: None,
            last_end_utc: None,
        });
        d.slots += 1;
        if r.fat_created.is_some() {
            s.fat_dated_slots += 1;
        }
        match r.state {
            SlotState::Recorded => s.recorded += 1,
            SlotState::Empty => s.empty += 1,
            SlotState::Residual => s.residual += 1,
            SlotState::Unrecognized => s.unrecognized += 1,
        }
        s.nal_total += r.nal.total;
        s.nal_nonstandard += r.nal.nonstandard;
        s.pictures += r.nal.pictures;
        s.payload_bytes += r.payload_len;
        if let Some(p) = r.payload_offset {
            *s.header_len_hist.entry(p).or_default() += 1;
        }
        if let Some(dlt) = r.fat_minus_header_end_s {
            *s.fat_delta_hist.entry(dlt).or_default() += 1;
        }
        if r.state != SlotState::Recorded {
            continue;
        }
        d.recorded += 1;
        let (st, en) = (r.header_start_utc, r.header_end_utc);
        if d.first_start_utc.is_none() {
            d.first_start_utc = st;
        }
        d.last_end_utc = en.or(d.last_end_utc);
        s.first_start_utc = match (s.first_start_utc, st) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        s.last_end_utc = match (s.last_end_utc, en) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        };
        if let (Some(a), Some(b)) = (r.header_start, r.header_end) {
            s.recorded_seconds += (b as i64 - a as i64).max(0);
        }
        if let (Some(p), Some(cur_start)) = (prev, r.header_start) {
            if let (Some(ps), Some(pe)) = (p.header_start, p.header_end) {
                if cur_start < ps {
                    s.slot_order_monotonic = false;
                }
                if cur_start < pe {
                    s.overlaps += 1;
                }
                let gap = cur_start as i64 - pe as i64;
                if gap > GAP_REPORT_S && s.gaps.len() < 100 {
                    s.gaps.push((r.slot, gap));
                }
            }
        }
        prev = Some(r);
    }
    s.est_fps = (s.recorded_seconds > 0).then(|| s.pictures as f64 / s.recorded_seconds as f64);
    s.dirs = dirs.into_values().collect();
    s
}

/// Byte range of the video payload inside one slot's bytes: from the first
/// valid H.265 start code (after the `luo ` header if present) to the last
/// non-zero byte.
pub fn slot_payload(data: &[u8]) -> Option<(usize, usize)> {
    let end = data.iter().rposition(|&b| b != 0)? + 1;
    let from = if data.len() >= 12 && &data[0..4] == MAGIC { 12 } else { 0 };
    let start = first_hevc_start(&data[..end], from)?;
    Some((start, end))
}

/// Parse the fixed `luo ` header fields: (start, end) Unix times.
pub fn slot_header(data: &[u8]) -> Option<(u32, u32)> {
    (data.len() >= 12 && &data[0..4] == MAGIC).then(|| (u32le(data, 4), u32le(data, 8)))
}

/// Keep only NAL units with a structurally valid H.265 header, re-framed
/// with 4-byte start codes (drops the recorder's interleaved non-standard units).
pub fn clean_hevc(payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len());
    for (a, b) in nal_spans(payload) {
        if is_valid_hevc_header(&payload[a..b]) {
            out.extend_from_slice(&[0, 0, 0, 1]);
            out.extend_from_slice(&payload[a..b]);
        }
    }
    out
}

/// "dir00012" -> 12 for prefix "dir"; "file0003.dat" -> 3 for ("file", ".dat").
fn numbered(name: &str, prefix: &str, suffix: &str) -> Option<u32> {
    let lower = name.to_ascii_lowercase();
    let mid = lower.strip_prefix(prefix)?.strip_suffix(suffix)?;
    if mid.is_empty() || !mid.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    mid.parse().ok()
}

fn u32le(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes([b[o], b[o + 1], b[o + 2], b[o + 3]])
}

/// Map the file range [start, start+len) onto disk extents.
fn sub_extents(map: &FileMap, start: u64, len: u64) -> Vec<Extent> {
    let end = start + len;
    map.extents
        .iter()
        .filter_map(|x| {
            let a = start.max(x.file_offset);
            let b = end.min(x.file_offset + x.len);
            (a < b).then(|| Extent { file_offset: a, disk_offset: x.disk_offset + (a - x.file_offset), len: b - a })
        })
        .collect()
}

/// A start code followed by a structurally valid H.265 NAL header:
/// forbidden_zero_bit = 0, nuh_layer_id = 0, nuh_temporal_id_plus1 != 0.
fn is_valid_hevc_header(nal: &[u8]) -> bool {
    if nal.len() < 2 {
        return false;
    }
    let (h0, h1) = (nal[0], nal[1]);
    let layer_id = ((h0 & 1) << 5) | (h1 >> 3);
    h0 & 0x80 == 0 && layer_id == 0 && h1 & 0x07 != 0
}

/// First start code at or after `from` that begins a valid H.265 NAL,
/// preferring a VPS. Returns the offset of the start code (4-byte form when
/// a zero byte precedes it).
fn first_hevc_start(data: &[u8], from: usize) -> Option<usize> {
    if from >= data.len() {
        return None;
    }
    let region = &data[from..];
    let back = |p: usize| if p > 0 && region[p - 1] == 0 { p - 1 } else { p };
    if let Some(p) = memmem::find(region, &[0, 0, 1, 0x40, 0x01]) {
        return Some(from + back(p));
    }
    memmem::find_iter(region, &[0, 0, 1])
        .find(|&p| is_valid_hevc_header(&region[p + 3..]))
        .map(|p| from + back(p))
}

/// NAL unit byte ranges (header..end, start codes and trailing zeros excluded).
fn nal_spans(payload: &[u8]) -> Vec<(usize, usize)> {
    let starts: Vec<usize> = memmem::find_iter(payload, &[0, 0, 1]).collect();
    let mut out = Vec::with_capacity(starts.len());
    for (i, &p) in starts.iter().enumerate() {
        let a = p + 3;
        let mut b = starts.get(i + 1).copied().unwrap_or(payload.len());
        while b > a && payload[b - 1] == 0 {
            b -= 1;
        }
        if b > a {
            out.push((a, b));
        }
    }
    out
}

fn nal_stats(payload: &[u8]) -> (NalStats, BTreeSet<String>) {
    let mut st = NalStats::default();
    let mut sps = BTreeSet::new();
    for (a, b) in nal_spans(payload) {
        let nal = &payload[a..b];
        st.total += 1;
        if !is_valid_hevc_header(nal) {
            st.nonstandard += 1;
            *st.nonstandard_first_byte.entry(format!("{:02x}", nal[0])).or_default() += 1;
            continue;
        }
        let t = (nal[0] >> 1) & 0x3F;
        *st.types.entry(t).or_default() += 1;
        match t {
            32 => st.vps += 1,
            33 => {
                st.sps += 1;
                sps.insert(hex::encode(&Sha256::digest(nal)[..6]));
            }
            34 => st.pps += 1,
            19 | 20 => st.idr += 1,
            21 => st.cra += 1,
            _ => {}
        }
        // VCL units: first_slice_segment_in_pic_flag is the first slice-header bit.
        if t <= 31 && nal.len() > 2 && nal[2] & 0x80 != 0 {
            st.pictures += 1;
        }
    }
    (st, sps)
}

struct HashingWriter<'w> {
    inner: &'w mut dyn Write,
    md5: Md5,
    sha: Sha256,
    bytes: u64,
}

impl<'w> HashingWriter<'w> {
    fn new(inner: &'w mut dyn Write) -> Self {
        Self { inner, md5: Md5::new(), sha: Sha256::new(), bytes: 0 }
    }
    fn finish(self) -> (u64, String, String) {
        (self.bytes, hex::encode(self.md5.finalize()), hex::encode(self.sha.finalize()))
    }
}

impl Write for HashingWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.md5.update(&buf[..n]);
        self.sha.update(&buf[..n]);
        self.bytes += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hevc_header_validation() {
        assert!(is_valid_hevc_header(&[0x40, 0x01])); // VPS
        assert!(is_valid_hevc_header(&[0x26, 0x01])); // IDR_W_RADL
        assert!(is_valid_hevc_header(&[0x00, 0x01])); // TRAIL_N is legal
        assert!(!is_valid_hevc_header(&[0x00, 0x00])); // temporal id 0: invalid
        assert!(!is_valid_hevc_header(&[0x80, 0x01])); // forbidden bit
    }

    #[test]
    fn finds_vps_after_header_not_inside_it() {
        // Header contains "00 00 00 01" as a LE integer field; VPS comes later.
        let mut slot = b"luo ".to_vec();
        slot.extend_from_slice(&[0, 0, 0, 0, 1, 0, 0, 0]);
        slot.extend_from_slice(&[0, 0, 0, 1, 0, 0, 0, 0]);
        slot.resize(0x40, 0);
        slot.extend_from_slice(&[0, 0, 0, 1, 0x40, 0x01, 0x0C]);
        assert_eq!(first_hevc_start(&slot, 12), Some(0x40));
    }

    #[test]
    fn nal_stats_count_types_and_nonstandard() {
        let mut p = Vec::new();
        for nal in [&[0x40u8, 0x01, 0xAA][..], &[0x42, 0x01, 0xBB], &[0x44, 0x01, 0xCC], &[0x26, 0x01, 0x80, 0x11], &[0x00, 0x00, 0x99], &[0x02, 0x01, 0x80]] {
            p.extend_from_slice(&[0, 0, 0, 1]);
            p.extend_from_slice(nal);
        }
        let (st, sps) = nal_stats(&p);
        assert_eq!((st.total, st.vps, st.sps, st.pps, st.idr), (6, 1, 1, 1, 1));
        assert_eq!(st.nonstandard, 1);
        assert_eq!(st.pictures, 2);
        assert_eq!(sps.len(), 1);
    }

    #[test]
    fn numbered_names() {
        assert_eq!(numbered("dir00012", "dir", ""), Some(12));
        assert_eq!(numbered("FILE0003.DAT", "file", ".dat"), Some(3));
        assert_eq!(numbered("index.bin", "file", ".dat"), None);
    }
}
