//! Independent timestamp anchors: NTP sync logs, ENF, solar.

pub mod ntp;
pub mod solar;

pub use ntp::{extract_ntp_events, NtpEvent};
pub use solar::{solar_time_estimate, SolarEstimate};
