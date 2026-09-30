//! # SATYA Temporal Trust Engine
//!
//! Per-frame timestamp estimation with calibrated 95% credible intervals.

pub mod claims;
pub mod collapse;
pub mod fusion;
pub mod calibration;

pub use claims::{Claim, Source};
pub use collapse::collapse_same_clock;
pub use fusion::{to_claim, fuse, fuse_with_diagnostics, FusedTimestamp};
pub use calibration::{coverage, median_error, reliability_diagram, ReliabilityBin};
