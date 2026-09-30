use satya_core::*;
use crate::nal_scan;

pub struct MatrixFs;

impl DvrFileSystem for MatrixFs {
    fn identify(image: &[u8]) -> Option<DeviceFingerprint> {
        if image.len() < 512 { return None; }
        if image.windows(7).take(256).any(|w| w == b"SATATYA") {
            return Some(DeviceFingerprint {
                oem: Oem::Matrix,
                model: None,
                firmware: None,
                block_size: None,
                confidence: 0.95,
            });
        }
        if image.windows(4).take(4096).any(|w| w == b"AVS\x00") {
            return Some(DeviceFingerprint {
                oem: Oem::Matrix,
                model: None,
                firmware: None,
                block_size: None,
                confidence: 0.8,
            });
        }
        None
    }

    fn enumerate_frames(image: &[u8]) -> Result<Vec<RecoveredFrame>> {
        Ok(nal_scan::scan_nal_units(image))
    }
}
