//! Dahua DHFS4.1 parser.
//!
//! Offsets sourced from the public `dhfs_extractor/dhfs41.py` implementation
//! and NaiveTomcat's blog analysis. All offsets are documented below and
//! should be verified against a real disk image before relying on results.

use satya_core::*;
use crate::nal_scan;

pub struct DahuaFs;

const DHFS_MAGIC: &[u8] = b"DHFS4.1";
const DHAV_MAGIC: u32 = 0x44484156; // "DHAV"
const DHAV_HDR_SIZE: usize = 22;
const SECTOR: u64 = 512;

// Partition table: located at LBA 30 (offset 0x3C00).
// After a 0x34-byte preamble, entries follow, 64 bytes each.
const PART_TABLE_OFF: usize = 0x3C00;
const PART_TABLE_PREAMBLE: usize = 0x34;
const PART_ENTRY_SIZE: usize = 64;

// Terminator bytes: 0xAA 0x55 0xAA 0x55
const PART_TERMINATOR: [u8; 4] = [0xAA, 0x55, 0xAA, 0x55];

#[derive(Debug, Clone, Copy)]
struct DhfsPartition {
    sb_offset: u64,    // in bytes
    part_offset: u64,  // in bytes
    blk_size: u32,
    frag_size: u32,
}

/// Decode Dahua's bit-packed 32-bit timestamp.
/// Bits: YYYYYY MMMM DDDDD HHHHH MMMMMM SSSSSS
#[allow(dead_code)]
fn decode_dahua_ts(ts: u32) -> Option<chrono::DateTime<chrono::Utc>> {
    use chrono::TimeZone;
    let year   = 2000 + ((ts >> 26) & 0x3F) as i32;
    let month  = ((ts >> 22) & 0x0F) as u32;
    let day    = ((ts >> 17) & 0x1F) as u32;
    let hour   = ((ts >> 12) & 0x1F) as u32;
    let minute = ((ts >> 6)  & 0x3F) as u32;
    let second = (ts & 0x3F) as u32;

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) { return None; }
    chrono::Utc.with_ymd_and_hms(year, month, day, hour, minute, second).single()
}

fn parse_partition_table(image: &[u8]) -> Vec<DhfsPartition> {
    let mut parts = Vec::new();
    let start = PART_TABLE_OFF + PART_TABLE_PREAMBLE;

    if start + PART_ENTRY_SIZE > image.len() {
        return parts;
    }

    let mut off = start;
    while off + PART_ENTRY_SIZE <= image.len() {
        let entry = &image[off..off + PART_ENTRY_SIZE];

        if entry[0..4] == PART_TERMINATOR { break; }

        // Bytes 20..24: SB_OFFS in blocks (u32 LE)
        // Bytes 48..56: PART_OFFS in blocks (u64 LE)
        let sb_blocks   = u32::from_le_bytes(entry[20..24].try_into().unwrap());
        let part_blocks = u64::from_le_bytes(entry[48..56].try_into().unwrap());

        if sb_blocks == 0 && part_blocks == 0 { break; }

        parts.push(DhfsPartition {
            sb_offset:   sb_blocks   as u64 * SECTOR,
            part_offset: part_blocks      * SECTOR,
            blk_size: 0,
            frag_size: 0,
        });
        off += PART_ENTRY_SIZE;
    }
    parts
}

fn read_superblock(image: &[u8], part: &mut DhfsPartition) {
    let sb = (part.part_offset + part.sb_offset) as usize;
    if sb + 0x34 > image.len() { return; }
    // Bytes 0x2c..0x30: BLK_SIZE (u32 LE)
    // Bytes 0x30..0x34: FRAG_SIZE (u32 LE)
    part.blk_size  = u32::from_le_bytes(image[sb + 0x2c..sb + 0x30].try_into().unwrap());
    part.frag_size = u32::from_le_bytes(image[sb + 0x30..sb + 0x34].try_into().unwrap());
}

/// Carve DHAV frames from a region of memory.
fn carve_dhav(region: &[u8], base_offset: u64, frames: &mut Vec<RecoveredFrame>) {
    let magic_bytes = DHAV_MAGIC.to_le_bytes();
    let mut off = 0usize;

    while off + DHAV_HDR_SIZE <= region.len() {
        let Some(pos) = memchr::memmem::find(&region[off..], &magic_bytes) else {
            break;
        };
        let abs = off + pos;

        if abs + DHAV_HDR_SIZE > region.len() { break; }

        let hdr = &region[abs..abs + DHAV_HDR_SIZE];
        let frame_type  = hdr[4];
        let _channel    = hdr[5].wrapping_add(1); // actual = value + 1
        let ts          = u32::from_le_bytes(hdr[6..10].try_into().unwrap());
        let _frame_num  = u32::from_le_bytes(hdr[10..14].try_into().unwrap());
        let frame_size  = u32::from_le_bytes(hdr[14..18].try_into().unwrap());
        let _tail_size  = u32::from_le_bytes(hdr[18..22].try_into().unwrap());

        // Frame type 0 = video
        if frame_type == 0
            && frame_size > 0
            && frame_size < 10_000_000
            && (abs + DHAV_HDR_SIZE + frame_size as usize) <= region.len()
            {
                let claimed = chrono::DateTime::from_timestamp(ts as i64, 0)
                .unwrap_or_default();

                frames.push(RecoveredFrame {
                    offset: base_offset + abs as u64,
                    length: (DHAV_HDR_SIZE + frame_size as usize) as u32,
                            codec: Codec::H264,
                            claims: vec![TimestampClaim {
                                frame_offset: base_offset + abs as u64,
                                claimed_utc: claimed,
                                source: TimestampSource::FrameHeader,
                                confidence: 0.75,
                            }],
                            recovery_source: RecoverySource::Allocated,
                            recovery_confidence: 0.9,
                });
                off = abs + DHAV_HDR_SIZE + frame_size as usize;
            } else {
                off = abs + 4;
            }
    }
}

impl DvrFileSystem for DahuaFs {
    fn identify(image: &[u8]) -> Option<DeviceFingerprint> {
        if image.len() < 16 { return None; }
        if image.starts_with(DHFS_MAGIC) {
            return Some(DeviceFingerprint {
                oem: Oem::Dahua,
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

        // 1. Parse partition table
        let mut parts = parse_partition_table(image);

        // 2. Fill in superblock fields
        for p in &mut parts {
            read_superblock(image, p);
        }

        // 3. Carve DHAV frames within each partition's data region
        for p in &parts {
            let start = p.part_offset as usize;
            if start >= image.len() { continue; }
            carve_dhav(&image[start..], p.part_offset, &mut frames);
        }

        // 4. Fallback: generic NAL carving across the whole image
        let existing: std::collections::HashSet<u64> =
        frames.iter().map(|f| f.offset).collect();
        let mut carved = nal_scan::scan_nal_units(image);
        carved.retain(|f| !existing.contains(&f.offset));
        frames.extend(carved);

        frames.sort_by_key(|f| f.offset);
        Ok(frames)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_known_timestamp() {
        // Build a timestamp for 2024-03-15 14:23:11
        let year: u32   = 24;
        let month: u32  = 3;
        let day: u32    = 15;
        let hour: u32   = 14;
        let minute: u32 = 23;
        let second: u32 = 11;
        let ts = (year << 26) | (month << 22) | (day << 17)
        | (hour << 12) | (minute << 6) | second;

        let dt = decode_dahua_ts(ts).unwrap();
        assert_eq!(dt.format("%Y-%m-%d %H:%M:%S").to_string(),
                   "2024-03-15 14:23:11");
    }
}
