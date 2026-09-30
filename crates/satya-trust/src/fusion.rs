use crate::claims::{Claim, Source};
use crate::collapse::collapse_same_clock;
use serde::{Deserialize, Serialize};

/// 95% two-sided z-score for a standard normal.
pub const Z_95: f64 = 1.959_963_984_540_054;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FusedTimestamp {
    pub mean_s: f64,
    pub sigma_s: f64,
    pub ci_lo_95_s: f64,
    pub ci_hi_95_s: f64,
    pub effective_sources: Vec<Source>,
    pub original_sources: Vec<Source>,
    pub n_input_claims: usize,
    pub n_after_collapse: usize,
    pub collapsed: bool,
}

impl FusedTimestamp {
    pub fn mean_iso(&self) -> String {
        chrono::DateTime::from_timestamp(self.mean_s as i64, 0)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
    }
}

/// Build a claim from a raw extraction. Confidence floor prevents
/// explosion if a source reports near-zero certainty.
pub fn to_claim(mean_s: f64, source: Source, confidence: f64) -> Claim {
    let c = confidence.clamp(0.05, 1.0);
    let sigma = source.base_sigma() / c;
    Claim::new(mean_s, sigma, source, c)
}

fn posterior(claims: &[Claim]) -> (f64, f64) {
    let mut prec_total = 0.0;
    let mut num = 0.0;
    for c in claims {
        let p = 1.0 / (c.sigma_s * c.sigma_s);
        prec_total += p;
        num += p * c.mean_s;
    }
    (num / prec_total, (1.0 / prec_total).sqrt())
}

/// Fuse timestamp claims. Returns (mean_s, lo_95_s, hi_95_s).
pub fn fuse(claims: &[Claim]) -> (f64, f64, f64) {
    let ft = fuse_with_diagnostics(claims);
    (ft.mean_s, ft.ci_lo_95_s, ft.ci_hi_95_s)
}

pub fn fuse_with_diagnostics(claims: &[Claim]) -> FusedTimestamp {
    let original_sources = unique_sources(claims);
    let collapsed = collapse_same_clock(claims);
    let effective_sources = unique_sources(&collapsed);

    if collapsed.is_empty() {
        return FusedTimestamp {
            mean_s: 0.0, sigma_s: 0.0,
            ci_lo_95_s: 0.0, ci_hi_95_s: 0.0,
            effective_sources, original_sources,
            n_input_claims: claims.len(), n_after_collapse: 0,
            collapsed: false,
        };
    }

    let (mean, sigma) = posterior(&collapsed);
    FusedTimestamp {
        mean_s: mean,
        sigma_s: sigma,
        ci_lo_95_s: mean - Z_95 * sigma,
        ci_hi_95_s: mean + Z_95 * sigma,
        effective_sources,
        original_sources,
        n_input_claims: claims.len(),
        n_after_collapse: collapsed.len(),
        collapsed: collapsed.len() < claims.len(),
    }
}

fn unique_sources(claims: &[Claim]) -> Vec<Source> {
    let mut v: Vec<Source> = claims.iter().map(|c| c.source).collect();
    v.sort_by_key(|s| format!("{:?}", s));
    v.dedup();
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::claims::Source;

    #[test]
    fn ntp_tightens_interval() {
        let dvr_only = vec![to_claim(1000.0, Source::FrameHeader, 0.8)];
        let with_ntp = vec![
            to_claim(1000.0, Source::FrameHeader, 0.8),
            to_claim(1010.0, Source::NtpSyncLog, 0.95),
        ];
        let (_, lo1, hi1) = fuse(&dvr_only);
        let (_, lo2, hi2) = fuse(&with_ntp);
        assert!((hi2 - lo2) < (hi1 - lo1), "NTP anchor must tighten the CI");
    }

    #[test]
    fn collapse_prevents_overconfidence() {
        // Three DVR sources agreeing should NOT beat a single NTP anchor.
        let dvr_votes = vec![
            to_claim(1000.0, Source::FrameHeader, 0.9),
            to_claim(1000.5, Source::OnScreenOcr, 0.9),
            to_claim(1000.2, Source::DeviceLog, 0.9),
        ];
        let ntp_only = vec![to_claim(1010.0, Source::NtpSyncLog, 0.95)];

        let ft_dvr = fuse_with_diagnostics(&dvr_votes);
        let ft_ntp = fuse_with_diagnostics(&ntp_only);

        // DVR collapse should be *wider* than NTP despite having 3 votes.
        assert!(ft_dvr.sigma_s > ft_ntp.sigma_s);
    }

    #[test]
    fn confidence_floor_prevents_explosion() {
        let c = to_claim(1000.0, Source::FrameHeader, 0.0);
        assert!(c.sigma_s.is_finite());
        assert!(c.sigma_s < 2000.0);
    }
}
