//! Locate the contiguous video payload in a DVR image.
//!
//! Rather than carving every NAL unit boundary (which risks splitting
//! a NAL in half and corrupting the stream), we identify the start and
//! end offsets of the video region and let FFmpeg demux from there.

use satya_core::*;

/// Returns (start, end) byte offsets of the video payload.
pub fn locate(image: &[u8], oem: Oem) -> Option<(usize, usize)> {
    match oem {
        Oem::Hikvision => locate_hikvision(image),
        Oem::Dahua => locate_dahua(image),
        Oem::Wfs => locate_wfs(image),
        Oem::CpPlus => locate_cpplus(image),
        Oem::Honeywell => locate_honeywell(image),
        Oem::Uniview => locate_uniview(image),
        Oem::TpLink => locate_tplink(image),
        Oem::Godrej => locate_godrej(image),
        Oem::Matrix => locate_matrix(image),
        // JUAN payloads live in many FAT32 slots; use juan::JuanVolume::export_hevc.
        Oem::Juan => None,
        Oem::Unknown => None,
    }
}

/// Find the first NAL start code in a slice, then return a range
/// from there to the end of the buffer.
fn from_first_nal(image: &[u8], search_start: usize) -> Option<(usize, usize)> {
    if search_start >= image.len() { return None; }
    let region = &image[search_start..];
    let nal4 = [0u8, 0, 0, 1];
    let nal3 = [0u8, 0, 1];
    let pos4 = memchr::memmem::find(region, &nal4);
    let pos3 = memchr::memmem::find(region, &nal3);
    let pos = match (pos4, pos3) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }?;
    Some((search_start + pos, image.len()))
}

fn locate_hikvision(image: &[u8]) -> Option<(usize, usize)> {
    // Master sector (2048) + log area (from 0x268) + video
    if image.len() < 0x270 { return None; }
    let log_off = u64::from_le_bytes(image[0x260..0x268].try_into().ok()?) as usize;
    let log_size = u64::from_le_bytes(image[0x268..0x270].try_into().ok()?) as usize;
    let video_start = if log_off > 0 && log_size > 0 {
        log_off + log_size
    } else {
        2048
    };
    from_first_nal(image, video_start)
}

fn locate_dahua(image: &[u8]) -> Option<(usize, usize)> {
    // Skip to partition offset (0x10000 by convention)
    from_first_nal(image, 0x10000)
}

fn locate_wfs(image: &[u8]) -> Option<(usize, usize)> {
    from_first_nal(image, 1024)
}

fn locate_cpplus(image: &[u8]) -> Option<(usize, usize)> {
    from_first_nal(image, 1024)
}

fn locate_honeywell(image: &[u8]) -> Option<(usize, usize)> {
    from_first_nal(image, 1024)
}

fn locate_uniview(image: &[u8]) -> Option<(usize, usize)> {
    from_first_nal(image, 512)
}

fn locate_tplink(image: &[u8]) -> Option<(usize, usize)> {
    from_first_nal(image, 2048)
}

fn locate_godrej(image: &[u8]) -> Option<(usize, usize)> {
    from_first_nal(image, 1536)
}

fn locate_matrix(image: &[u8]) -> Option<(usize, usize)> {
    from_first_nal(image, 512)
}
