use serde::{Deserialize, Serialize};

pub fn coverage(intervals: &[(f64, f64)], truth: &[f64]) -> f64 {
    assert_eq!(intervals.len(), truth.len());
    if intervals.is_empty() {
        return 0.0;
    }
    let hits = intervals
    .iter()
    .zip(truth)
    .filter(|(&(lo, hi), &gt)| lo <= gt && gt <= hi)
    .count();
    hits as f64 / intervals.len() as f64
}

pub fn median_error(means: &[f64], truth: &[f64]) -> f64 {
    assert_eq!(means.len(), truth.len());
    if means.is_empty() {
        return 0.0;
    }
    let mut errs: Vec<f64> = means
    .iter()
    .zip(truth)
    .map(|(&m, &g)| (m - g).abs())
    .collect();
    errs.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = errs.len();
    if n % 2 == 1 {
        errs[n / 2]
    } else {
        0.5 * (errs[n / 2 - 1] + errs[n / 2])
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReliabilityBin {
    pub z: f64,
    pub nominal: f64,
    pub empirical: f64,
    pub n: usize,
}

pub fn reliability_diagram(
    means: &[f64],
    sigmas: &[f64],
    truth: &[f64],
    n_bins: usize,
) -> Vec<ReliabilityBin> {
    let n = means.len();
    let mut out = Vec::with_capacity(n_bins);
    for k in 1..=n_bins {
        let z = k as f64 * 0.5;
        let hits = means
        .iter()
        .zip(sigmas)
        .zip(truth)
        .filter(|((&m, &s), &g)| (g - m).abs() <= z * s)
        .count();
        let nominal = erf(z / std::f64::consts::SQRT_2);
        out.push(ReliabilityBin {
            z,
            nominal,
            empirical: if n == 0 { 0.0 } else { hits as f64 / n as f64 },
            n,
        });
    }
    out
}

fn erf(x: f64) -> f64 {
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let x = x.abs();
    let t = 1.0 / (1.0 + 0.3275911 * x);
    let y = 1.0
    - (((((1.061405429 * t - 1.453152027) * t) + 1.421413741) * t - 0.284496736)
    * t
    + 0.254829592)
    * t
    * (-x * x).exp();
    sign * y
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fusion::Z_95;

    #[test]
    fn coverage_perfect() {
        let intervals = vec![(0.0, 10.0), (10.0, 20.0)];
        let truth = vec![5.0, 15.0];
        assert_eq!(coverage(&intervals, &truth), 1.0);
    }

    #[test]
    fn coverage_zero() {
        let intervals = vec![(0.0, 1.0), (0.0, 1.0)];
        let truth = vec![5.0, 15.0];
        assert_eq!(coverage(&intervals, &truth), 0.0);
    }

    #[test]
    fn z_95_matches_standard() {
        assert!((Z_95 - 1.959963984540054).abs() < 1e-12);
    }
}
