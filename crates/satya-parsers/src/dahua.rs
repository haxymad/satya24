use satya_core::*;
use crate::nal_scan;

pub struct DahuaFs;

const SIG: &[u8] = b"DHFS4.1";
const DHFS_FRAME_MAGIC: u32 = 0x44484653; // "DHFS"

impl DvrFileSystem for DahuaFs {
    fn identify(image: &[u8]) -> Option<DeviceFingerprint> {
        if image.len() < 16 { return None; }
        if &image[0..8] == SIG {
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

        // Parse DHFS frames starting after reserved sectors 1-33
        let mut off = 34 * 512;
        while off + 20 <= image.len() {
            let magic = u32::from_le_bytes(
                image[off..off+4].try_into().unwrap()
            );
            if magic == DHFS_FRAME_MAGIC {
                let ts = u32::from_le_bytes(
                    image[off+6..off+10].try_into().unwrap()
                );
                let length = u32::from_le_bytes(
                    image[off+10..off+14].try_into().unwrap()
                );
                let footer = u32::from_le_bytes(
                    image[off+14..off+18].try_into().unwrap()
                );

                // Dual-signature validation
                let conf = if footer == magic { 0.95 } else { 0.5 };

                if length > 0 && length < 10_000_000 && off + 20 + length as usize <= image.len() {
                    frames.push(RecoveredFrame {
                        offset: off as u64,
                        length,
                        codec: Codec::H264,
                        claims: vec![TimestampClaim {
                            frame_offset: off as u64,
                            claimed_utc: chrono::DateTime::from_timestamp(
                                ts as i64, 0
                            ).unwrap_or_default(),
                                source: TimestampSource::FrameHeader,
                                confidence: 0.7,
                        }],
                        recovery_source: RecoverySource::Allocated,
                        recovery_confidence: conf,
                    });
                    off += 20 + length as usize;
                    continue;
                }
            }
            off += 512;
        }

        let mut carved = nal_scan::scan_nal_units(image);
        frames.append(&mut carved);
        Ok(frames)
    }
}
