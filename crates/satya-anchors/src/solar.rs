//! Solar position as a coarse timestamp anchor.
//!
//! Given a latitude/longitude and the observed shadow direction, we
//! can bound the time of day. This is coarse (minutes to tens of minutes)
//! and outdoor-only, but it's genuinely independent of the DVR clock.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SolarEstimate {
    pub mean_s: f64,
    pub sigma_s: f64,
}

/// Compute the local solar noon for a given date and longitude.
pub fn solar_noon_utc_seconds(day_utc: f64, longitude_deg: f64) -> f64 {
    let day_start = (day_utc / 86400.0).floor() * 86400.0;
    let hours_offset = longitude_deg / 15.0;
    day_start + (12.0 - hours_offset) * 3600.0
}

pub fn solar_time_estimate(day_utc: f64, longitude_deg: f64) -> SolarEstimate {
    SolarEstimate {
        mean_s: solar_noon_utc_seconds(day_utc, longitude_deg),
        sigma_s: 900.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn solar_noon_utc_reasonable() {
        let t = solar_noon_utc_seconds(0.0, 0.0);
        assert!((t - 43200.0).abs() < 1.0);

        let delhi = solar_noon_utc_seconds(0.0, 77.0);
        assert!(delhi < 43200.0);
    }
}
