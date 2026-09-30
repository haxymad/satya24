//! NTP sync log parsing.
//!
//! When a DVR syncs to NTP, it logs the measured offset. Reconstructing
//! the true time from that log gives a genuinely independent anchor —
//! the DVR is measuring itself against an external reference.

use chrono::{NaiveDateTime, TimeZone, Utc};
use regex::Regex;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NtpEvent {
    pub dvr_time_s: f64,
    pub offset_s: f64,
    pub true_time_s: f64,
}

fn iso_pattern() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?i)(?P<ts>\d{4}-\d{2}-\d{2}[ T]\d{2}:\d{2}:\d{2}).*?offset[:\s]+(?P<off>[+-]?\d+\.\d+)",
        )
        .unwrap()
    })
}

fn epoch_pattern() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(
            r"(?i)NTP[_ ]?SYNC.*?time=(?P<ts>\d+).*?offset=(?P<off>[+-]?\d+\.\d+)",
        )
        .unwrap()
    })
}

pub fn extract_ntp_events(log: &str) -> Vec<NtpEvent> {
    let mut out: Vec<NtpEvent> = Vec::new();

    for cap in iso_pattern().captures_iter(log) {
        let ts_raw = &cap["ts"];
        let off: f64 = match cap["off"].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let normalized = ts_raw.replace('T', " ");
        let dt = match NaiveDateTime::parse_from_str(&normalized, "%Y-%m-%d %H:%M:%S") {
            Ok(d) => d,
            Err(_) => continue,
        };
        let dvr_time = Utc.from_utc_datetime(&dt).timestamp() as f64;
        out.push(NtpEvent {
            dvr_time_s: dvr_time,
            offset_s: off,
            true_time_s: dvr_time + off,
        });
    }

    for cap in epoch_pattern().captures_iter(log) {
        let dvr_time: f64 = match cap["ts"].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let off: f64 = match cap["off"].parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        out.push(NtpEvent {
            dvr_time_s: dvr_time,
            offset_s: off,
            true_time_s: dvr_time + off,
        });
    }

    out.sort_by(|a, b| a.dvr_time_s.partial_cmp(&b.dvr_time_s).unwrap());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_iso_format() {
        let log = "2024-03-15 14:23:11 NTP sync offset: +0.834s\n";
        let events = extract_ntp_events(log);
        assert_eq!(events.len(), 1);
        assert!((events[0].offset_s - 0.834).abs() < 1e-9);
    }

    #[test]
    fn parses_epoch_format() {
        let log = "NTP_SYNC time=1710512591 offset=-1.23\n";
        let events = extract_ntp_events(log);
        assert_eq!(events.len(), 1);
        assert!((events[0].true_time_s - 1710512589.77).abs() < 0.01);
    }
}
