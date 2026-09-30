pub mod hikvision;
pub mod dahua;
pub mod cpplus;
pub mod honeywell;
pub mod uniview;
pub mod tplink;
pub mod godrej;
pub mod matrix;
pub mod nal_scan;

use satya_core::*;

/// Dispatch to correct parser based on identification.
pub fn identify_device(image: &[u8]) -> Option<DeviceFingerprint> {
    hikvision::HikvisionFs::identify(image)
    .or_else(|| dahua::DahuaFs::identify(image))
    .or_else(|| matrix::MatrixFs::identify(image))
    .or_else(|| honeywell::HoneywellFs::identify(image))
    .or_else(|| uniview::UniviewFs::identify(image))
    .or_else(|| tplink::TpLinkFs::identify(image))
    .or_else(|| godrej::GodrejFs::identify(image))
    .or_else(|| cpplus::CpPlusFs::identify(image))
}

pub fn enumerate_frames(image: &[u8], oem: Oem) -> Result<Vec<RecoveredFrame>> {
    match oem {
        Oem::Hikvision => hikvision::HikvisionFs::enumerate_frames(image),
        Oem::Dahua => dahua::DahuaFs::enumerate_frames(image),
        Oem::CpPlus => cpplus::CpPlusFs::enumerate_frames(image),
        Oem::Honeywell => honeywell::HoneywellFs::enumerate_frames(image),
        Oem::Uniview => uniview::UniviewFs::enumerate_frames(image),
        Oem::TpLink => tplink::TpLinkFs::enumerate_frames(image),
        Oem::Godrej => godrej::GodrejFs::enumerate_frames(image),
        Oem::Matrix => matrix::MatrixFs::enumerate_frames(image),
        Oem::Unknown => Err(DvrError::UnsupportedOem("unknown".into())),
    }
}
