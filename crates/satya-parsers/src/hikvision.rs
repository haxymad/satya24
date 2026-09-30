//! Hikvision file system parser.
//!
//! Master sector layout from the Wiley "Hikvision log records" paper
//! and the IPED PR #1776 (`gfd2020`). The RATS carver is the novelty:
//! commercial tools ignore log records, but they carry timestamp evidence.

use satya_core::*;
use crate::nal_scan;

pub struct HikvisionFs;

const SIG: &[u8] = b"HIKVISION@HANGZHOU";
const LOG_OFFSET_POS: usize = 0x260;
const LOG_SIZE_POS:   usize = 0x268;

/// RATS log record magic. From the Wiley paper: `52 41 54 53 01 00 00 00`.
const RATS_MAGIC: &[u8] = &[0x52, 0x41, 0x54, 0x53, 0x01, 0x00, 0x00, 0x00];
const RATS_RECORD_SIZE: usize = 16;

#[derive(Debug)]
struct HikMasterSector {
    log_offset: u64,
    log_size: u64,
}

impl HikMasterSector {
    fn parse(image: &[u8]) -> Option<Self> {
        if image.len() < LOG_SIZE_POS + 8 { return None; }
        if !image.starts_with(SIG) { return None; }

        let log_offset = u64::from_le_bytes(
            image[LOG_OFFSET_POS..LOG_OFFSET_POS + 8].try_into().ok()?,
        );
        let log_size = u64::from_le_bytes(
            image[LOG_SIZE_POS..LOG_SIZE_POS + 8].try_into().ok()?,
        );
        Some(Self { log_offset, log_size })
    }
}

#[derive(Debug, Clone, Copy)]
struct RatsRecord {
    offset: u64,
    timestamp: u32,
    major_type: u16,
    #[allow(dead_code)]
    minor_type: u16,
}
impl RatsRecord {
    fn parse(data: &[u8], abs_offset: u64) -> Option<Self> {
        if data.len() < RATS_RECORD_SIZE { return None; }
        if &data[0..8] != RATS_MAGIC { return None; }

        Some(Self {
            offset: abs_offset,
            timestamp:  u32::from_le_bytes(data[8..12].try_into().ok()?),
             major_type: u16::from_le_bytes(data[12..14].try_into().ok()?),
             minor_type: u16::from_le_bytes(data[14..16].try_into().ok()?),
        })
    }
}

fn carve_rats(image: &[u8]) -> Vec<RatsRecord> {
    let mut records = Vec::new();
    for pos in memchr::memmem::find_iter(image, RATS_MAGIC) {
        if let Some(r) = RatsRecord::parse(&image[pos..], pos as u64) {
            records.push(r);
        }
    }
    records
}

fn rats_to_frame(rec: &RatsRecord) -> Option<RecoveredFrame> {
    // Only video-related records (major_type = 0x0002) become frames.
    if rec.major_type != 0x0002 { return None; }

    let claimed = chrono::DateTime::from_timestamp(rec.timestamp as i64, 0)
    .unwrap_or_default();

    Some(RecoveredFrame {
        offset: rec.offset,
         length: 0,
         codec: Codec::H264,
         claims: vec![TimestampClaim {
             frame_offset: rec.offset,
             claimed_utc: claimed,
             source: TimestampSource::DeviceLog,
             confidence: 0.6, // DVR clock only; anchored later by trust engine
         }],
         recovery_source: RecoverySource::Allocated,
         recovery_confidence: 0.85,
    })
}

impl DvrFileSystem for HikvisionFs {
    fn identify(image: &[u8]) -> Option<DeviceFingerprint> {
        if image.len() < 2048 { return None; }
        if image.starts_with(SIG) {
            return Some(DeviceFingerprint {
                oem: Oem::Hikvision,
                model: None,
                firmware: None,
                block_size: None,
                confidence: 1.0,
            });
        }
        None
    }

    fn enumerate_frames(image: &[u8]) -> Result<Vec<RecoveredFrame>> {
        let mut frames = Vec::new();
        let mut seen_offsets = std::collections::HashSet::new();

        // 1. Parse the documented log area (if master sector is present).
        if let Some(m) = HikMasterSector::parse(image) {
            let start = m.log_offset as usize;
            let end = (start + m.log_size as usize).min(image.len());
            if start < end {
                for rec in carve_rats(&image[start..end]) {
                    if let Some(f) = rats_to_frame(&rec) {
                        seen_offsets.insert(f.offset);
                        frames.push(f);
                    }
                }
            }
        }

        // 2. Carve RATS across the WHOLE image — the IPED PR confirmed these
        //    appear OUTSIDE the documented log area. This is the novelty.
        for rec in carve_rats(image) {
            if !seen_offsets.contains(&rec.offset) {
                if let Some(f) = rats_to_frame(&rec) {
                    seen_offsets.insert(f.offset);
                    frames.push(f);
                }
            }
        }

        // 3. NAL carving for actual video payload.
        for f in nal_scan::scan_nal_units(image) {
            if !seen_offsets.contains(&f.offset) {
                seen_offsets.insert(f.offset);
                frames.push(f);
            }
        }

        frames.sort_by_key(|f| f.offset);
        Ok(frames)
    }
}
