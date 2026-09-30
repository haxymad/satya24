use satya_core::*;
use crate::nal_scan;

pub struct GodrejFs;

impl DvrFileSystem for GodrejFs {
    fn identify(image: &[u8]) -> Option<DeviceFingerprint> {
        if image.len() < 512 { return None; }
        let has_fat = &image[82..87] == b"FAT32";
        let has_gj = image.windows(6).take(4096).any(|w| w == b"GODREJ");
        if has_gj && has_fat {
            return Some(DeviceFingerprint {
                oem: Oem::Godrej,
                model: None,
                firmware: None,
                block_size: Some(512),
                        confidence: 0.9,
            });
        }
        None
    }

    fn enumerate_frames(image: &[u8]) -> Result<Vec<RecoveredFrame>> {
        // Godrej: FAT32 + H.264 + G.711A audio. NAL carving works.
        Ok(nal_scan::scan_nal_units(image))
    }
}
