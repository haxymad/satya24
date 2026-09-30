use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Source {
    NtpSyncLog,
    Enf,
    OnScreenOcr,
    DeviceLog,
    FrameHeader,
    Solar,
    DvrClockCollapsed,
}

impl Source {
    pub fn base_sigma(&self) -> f64 {
        match self {
            Source::NtpSyncLog => 2.0,
            Source::Enf => 0.5,
            Source::OnScreenOcr => 30.0,
            Source::DeviceLog => 10.0,
            Source::FrameHeader => 60.0,
            Source::Solar => 900.0,
            Source::DvrClockCollapsed => 0.0,
        }
    }

    pub fn is_dvr_clock(&self) -> bool {
        matches!(
            self,
            Source::FrameHeader | Source::OnScreenOcr | Source::DeviceLog
        )
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Claim {
    pub mean_s: f64,
    pub sigma_s: f64,
    pub source: Source,
    pub confidence: f64,
}

impl Claim {
    pub fn new(mean_s: f64, sigma_s: f64, source: Source, confidence: f64) -> Self {
        Self { mean_s, sigma_s, source, confidence }
    }
}
