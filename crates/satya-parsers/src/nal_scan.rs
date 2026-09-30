use memchr::memmem;
use satya_core::*;

/// Scan the whole image for H.264/H.265 NAL units.
/// This is the fallback for OEMs whose FS layout we can't fully parse.
pub fn scan_nal_units(image: &[u8]) -> Vec<RecoveredFrame> {
    let mut positions: Vec<usize> = Vec::new();

    // 4-byte start code: 00 00 00 01
    for pos in memmem::find_iter(image, &[0u8, 0, 0, 1]) {
        positions.push(pos);
    }
    // 3-byte start code: 00 00 01
    for pos in memmem::find_iter(image, &[0u8, 0, 1]) {
        if !positions.contains(&pos.saturating_sub(1)) {
            positions.push(pos);
        }
    }
    positions.sort_unstable();
    positions.dedup();

    let mut frames = Vec::new();
    for w in positions.windows(2) {
        let start = w[0];
        let end = w[1];
        let length = (end - start) as u32;
        if length < 5 || length > 10_000_000 { continue; }

        let nal_type = image[start + 4] & 0x1F;
        let confidence = match nal_type {
            5 => 0.95,       // IDR slice
            1 => 0.80,       // non-IDR slice
            7 | 8 => 0.70,   // SPS/PPS
            _ => 0.40,
        };

        frames.push(RecoveredFrame {
            offset: start as u64,
            length,
            codec: Codec::H264,
            claims: vec![],
            recovery_source: RecoverySource::Carved,
            recovery_confidence: confidence,
        });
    }
    frames
}
