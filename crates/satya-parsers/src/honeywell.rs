use satya_core::*;
use crate::nal_scan;

pub struct HoneywellFs;

impl DvrFileSystem for HoneywellFs {
    fn identify(image: &[u8]) -> Option<DeviceFingerprint> {
        if image.len() < 512 { return None; }
        let sig = b"HONEYWELL";
        if image.windows(sig.len()).take(256).any(|w| w == sig) {
            return Some(DeviceFingerprint {
                oem: Oem::Honeywell,
                model: None,
                firmware: None,
                block_size: None,
                confidence: 0.9,
            });
        }
        None
    }

    fn enumerate_frames(image: &[u8]) -> Result<Vec<RecoveredFrame>> {
        // Honeywell: 20-byte custom header + 6-byte NAL prefix.
        // We use generic NAL carving; the 20-byte offset is applied
        // when the header is present.
        Ok(nal_scan::scan_nal_units(image))
    }
}
