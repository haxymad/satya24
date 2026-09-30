use satya_core::*;
use crate::nal_scan;

pub struct UniviewFs;

impl DvrFileSystem for UniviewFs {
    fn identify(image: &[u8]) -> Option<DeviceFingerprint> {
        if image.len() < 512 { return None; }
        let sigs: [&[u8]; 3] = [b"UNV", b"UNVIEW", b"UNV-FS"];
        for sig in sigs {
            if image.windows(sig.len()).take(512).any(|w| w == sig) {
                return Some(DeviceFingerprint {
                    oem: Oem::Uniview,
                    model: None,
                    firmware: None,
                    block_size: None,
                    confidence: 0.85,
                });
            }
        }
        None
    }

    fn enumerate_frames(image: &[u8]) -> Result<Vec<RecoveredFrame>> {
        Ok(nal_scan::scan_nal_units(image))
    }
}
