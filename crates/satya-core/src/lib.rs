pub mod analysis;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum DvrError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("parse error at offset {offset}: {msg}")]
    Parse { offset: u64, msg: String },
    #[error("unsupported OEM: {0}")]
    UnsupportedOem(String),
    #[error("validation error: {0}")]
    Validation(String),
}

pub type Result<T> = std::result::Result<T, DvrError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Oem {
    Hikvision,
    Dahua,
    Wfs,
    CpPlus,
    Honeywell,
    Uniview,
    TpLink,
    Godrej,
    Matrix,
    Unknown,
}

impl Oem {
    pub fn as_str(&self) -> &'static str {
        match self {
            Oem::Hikvision => "hikvision",
            Oem::Dahua     => "dahua",
            Oem::Wfs       => "wfs",
            Oem::CpPlus    => "cpplus",
            Oem::Honeywell => "honeywell",
            Oem::Uniview   => "uniview",
            Oem::TpLink    => "tplink",
            Oem::Godrej    => "godrej",
            Oem::Matrix    => "matrix",
            Oem::Unknown   => "unknown",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TimestampSource {
    OnScreenOcr,
    FrameHeader,
    DeviceLog,
    NtpSyncLog,
    SeiMetadata,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimestampClaim {
    pub frame_offset: u64,
    pub claimed_utc: DateTime<Utc>,
    pub source: TimestampSource,
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Codec { H264, H265 }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RecoverySource {
    Allocated,
    Carved,
    Partial,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecoveredFrame {
    pub offset: u64,
    pub length: u32,
    pub codec: Codec,
    pub claims: Vec<TimestampClaim>,
    pub recovery_source: RecoverySource,
    pub recovery_confidence: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceFingerprint {
    pub oem: Oem,
    pub model: Option<String>,
    pub firmware: Option<String>,
    pub block_size: Option<u32>,
    pub confidence: f32,
}

pub trait DvrFileSystem {
    fn identify(image: &[u8]) -> Option<DeviceFingerprint>
    where
    Self: Sized;

    fn enumerate_frames(image: &[u8]) -> Result<Vec<RecoveredFrame>>
    where
    Self: Sized;
}
