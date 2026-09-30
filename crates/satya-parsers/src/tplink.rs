use satya_core::*;
use crate::nal_scan;

pub struct TpLinkFs;

impl DvrFileSystem for TpLinkFs {
    fn identify(image: &[u8]) -> Option<DeviceFingerprint> {
        if image.len() < 0x440 { return None; }
        // ext4 superblock magic 0xEF53 at offset 0x438
        let ext4_magic = u16::from_le_bytes([image[0x438], image[0x439]]);
        let has_tp = image.windows(4).take(512).any(|w| w == b"TPOS");

        if has_tp || ext4_magic == 0xEF53 {
            return Some(DeviceFingerprint {
                oem: Oem::TpLink,
                model: None,
                firmware: None,
                block_size: Some(4096),
                        confidence: if has_tp { 0.85 } else { 0.55 },
            });
        }
        None
    }

    fn enumerate_frames(image: &[u8]) -> Result<Vec<RecoveredFrame>> {
        Ok(nal_scan::scan_nal_units(image))
    }
}
