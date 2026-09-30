use satya_core::*;
use crate::nal_scan;

pub struct CpPlusFs;

impl DvrFileSystem for CpPlusFs {
    fn identify(image: &[u8]) -> Option<DeviceFingerprint> {
        if image.len() < 512 { return None; }
        // FAT32 or DAV signature
        let has_fat32 = image.len() > 90 && &image[82..87] == b"FAT32";
        let has_dav = image.windows(4).take(4096).any(|w| w == b"DAV\x00");
        if has_fat32 || has_dav {
            return Some(DeviceFingerprint {
                oem: Oem::CpPlus,
                model: None,
                firmware: None,
                block_size: Some(512),
                        confidence: if has_dav { 0.85 } else { 0.6 },
            });
        }
        None
    }

    fn enumerate_frames(image: &[u8]) -> Result<Vec<RecoveredFrame>> {
        // CP Plus uses FAT32/ext with DAV files. Without a real FS
        // parser, fall back to NAL carving.
        Ok(nal_scan::scan_nal_units(image))
    }
}
