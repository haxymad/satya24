use satya_core::*;
use crate::nal_scan;

pub struct HikvisionFs;

const SIG: &[u8] = b"HIKVISION@HANGZHOU";
const LOG_OFFSET_POS: usize = 0x260; // 608
const LOG_SIZE_POS: usize = 0x268;   // 616

impl DvrFileSystem for HikvisionFs {
    fn identify(image: &[u8]) -> Option<DeviceFingerprint> {
        if image.len() < 2048 { return None; }
        if image.windows(SIG.len()).take(4).any(|w| w == SIG) {
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

        // Parse master sector log table
        if image.len() >= 2048 {
            let log_off = u64::from_le_bytes(
                image[LOG_OFFSET_POS..LOG_OFFSET_POS+8].try_into().unwrap()
            ) as usize;
            let log_size = u64::from_le_bytes(
                image[LOG_SIZE_POS..LOG_SIZE_POS+8].try_into().unwrap()
            ) as usize;

            if log_off > 0 && log_off + log_size <= image.len() {
                let mut off = log_off;
                let end = log_off + log_size;
                while off + 16 <= end {
                    // Hikvision log record: magic "RATS" then timestamp
                    if &image[off..off+4] == b"RATS" {
                        let ts = u32::from_le_bytes(
                            image[off+8..off+12].try_into().unwrap()
                        );
                        let major = u16::from_le_bytes(
                            image[off+12..off+14].try_into().unwrap()
                        );
                        if major == 0x0002 {
                            // Video log
                            frames.push(RecoveredFrame {
                                offset: off as u64,
                                length: 0,
                                codec: Codec::H264,
                                claims: vec![TimestampClaim {
                                    frame_offset: off as u64,
                                    claimed_utc: chrono::DateTime::from_timestamp(
                                        ts as i64, 0
                                    ).unwrap_or_default(),
                                        source: TimestampSource::DeviceLog,
                                        confidence: 0.6,
                                }],
                                recovery_source: RecoverySource::Allocated,
                                recovery_confidence: 0.85,
                            });
                        }
                    }
                    off += 32;
                }
            }
        }

        // Also carve NAL units (catches deleted frames)
        let mut carved = nal_scan::scan_nal_units(image);
        frames.append(&mut carved);

        Ok(frames)
    }
}
