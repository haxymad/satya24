//! WFS0.4 / WFS0.5 parser.
//!
//! WFS is a file system used by many Chinese DVRs (XMEye, generic OEMs).
//! The only public extraction tool is the Sleuthkit 4.9 fork
//! (`gbatmobile/sleuthkit4.9.0-wfs`). This is a signature-only implementation
//! that falls back to NAL carving; the inode parser needs the Sleuthkit C
//! source to port fully.

use satya_core::*;
use crate::nal_scan;

pub struct WfsFs;

const WFS04: &[u8] = b"WFS0.4";
const WFS05: &[u8] = b"WFS0.5";
const PROBE_WINDOW: usize = 1024;

impl DvrFileSystem for WfsFs {
    fn identify(image: &[u8]) -> Option<DeviceFingerprint> {
        if image.len() < PROBE_WINDOW { return None; }

        let window = &image[..PROBE_WINDOW];
        let has_v4 = window.windows(WFS04.len()).any(|w| w == WFS04);
        let has_v5 = window.windows(WFS05.len()).any(|w| w == WFS05);

        if has_v4 || has_v5 {
            return Some(DeviceFingerprint {
                oem: Oem::Wfs,
                model: Some(if has_v5 { "WFS0.5" } else { "WFS0.4" }.into()),
                        firmware: None,
                        block_size: None,
                        confidence: 0.9,
            });
        }
        None
    }

    fn enumerate_frames(image: &[u8]) -> Result<Vec<RecoveredFrame>> {
        // Full WFS inode parser needs the Sleuthkit fork as reference.
        // For now, fall back to NAL carving.
        Ok(nal_scan::scan_nal_units(image))
    }
}
