// SPDX-License-Identifier: GPL-3.0-or-later

//! The regression gate (docs/architecture.md §11.4): a run passes when every
//! metric meets its absolute target from the PRD and has not regressed
//! beyond noise against the stored baseline for this architecture.

use serde::Deserialize;
use std::collections::BTreeMap;

/// Measured values by metric name. Booleans are stored as 1.0 / 0.0.
pub type Metrics = BTreeMap<String, f64>;

/// Limits for one metric (`bench/thresholds.toml`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Limit {
    /// Absolute target: the value must not exceed it.
    #[serde(default)]
    pub max: Option<f64>,
    /// Absolute target: the value must be at least this (e.g. 1 for "true").
    #[serde(default)]
    pub min: Option<f64>,
    /// Allowed growth over the baseline, as a factor (1.5 = +50 %).
    #[serde(default)]
    pub regress: Option<f64>,
    /// Absolute allowance on top of `regress`, for metrics near zero where
    /// a factor means nothing (milliseconds of noise, a few wakeups).
    #[serde(default)]
    pub slack: f64,
    /// What it is, for the report.
    #[serde(default)]
    pub unit: String,
}

/// `bench/thresholds.toml`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Thresholds {
    pub metrics: BTreeMap<String, Limit>,
}

/// One line of the report.
#[derive(Debug, Clone, PartialEq)]
pub struct Verdict {
    pub metric: String,
    pub value: Option<f64>,
    pub baseline: Option<f64>,
    pub ok: bool,
    pub why: String,
}

/// Checks `results` against `limits` and `baseline`. A metric listed in the
/// thresholds but missing from the results fails: a gate that silently
/// skips what it could not measure gates nothing.
pub fn check(results: &Metrics, baseline: &Metrics, limits: &Thresholds) -> Vec<Verdict> {
    limits
        .metrics
        .iter()
        .map(|(name, l)| {
            let value = results.get(name).copied();
            let base = baseline.get(name).copied();
            let mut why = Vec::new();
            match value {
                None => why.push("not measured".to_owned()),
                Some(v) => {
                    if let Some(max) = l.max
                        && v > max
                    {
                        why.push(format!("{v} above the target {max}"));
                    }
                    if let Some(min) = l.min
                        && v < min
                    {
                        why.push(format!("{v} below the target {min}"));
                    }
                    if let (Some(factor), Some(b)) = (l.regress, base) {
                        let allowed = b * factor + l.slack;
                        if v > allowed {
                            why.push(format!(
                                "{v} regressed from the baseline {b} (allowed up to {allowed:.2})"
                            ));
                        }
                    }
                }
            }
            Verdict {
                metric: name.clone(),
                value,
                baseline: base,
                ok: why.is_empty(),
                why: why.join("; "),
            }
        })
        .collect()
}

/// `metric=factor` pairs: multiply those results, to prove the gate fails on
/// a regression (A17 acceptance).
pub fn inject(results: &mut Metrics, spec: &str) -> Result<(), String> {
    let (name, factor) = spec
        .split_once('=')
        .ok_or_else(|| format!("expected metric=factor, got '{spec}'"))?;
    let factor: f64 = factor
        .parse()
        .map_err(|_| format!("'{factor}' is not a number"))?;
    let v = results
        .get_mut(name)
        .ok_or_else(|| format!("no metric '{name}' in the results"))?;
    *v *= factor;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> Thresholds {
        toml::from_str(
            r#"
[metrics.relink_p95_ms]
max = 100
regress = 1.5
slack = 5
unit = "ms"
[metrics.pss_kb]
max = 10240
regress = 1.2
[metrics.bit_exact]
min = 1
[metrics.added_latency_samples]
max = 0
"#,
        )
        .expect("thresholds")
    }

    fn metrics(pairs: &[(&str, f64)]) -> Metrics {
        pairs.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect()
    }

    fn base() -> Metrics {
        metrics(&[
            ("relink_p95_ms", 8.0),
            ("pss_kb", 4000.0),
            ("bit_exact", 1.0),
            ("added_latency_samples", 0.0),
        ])
    }

    fn failed(v: &[Verdict]) -> Vec<&str> {
        v.iter()
            .filter(|x| !x.ok)
            .map(|x| x.metric.as_str())
            .collect()
    }

    #[test]
    fn a_run_like_the_baseline_passes() {
        let mut r = base();
        r.insert("relink_p95_ms".into(), 12.0); // noise: 8 × 1.5 + 5 = 17
        assert!(failed(&check(&r, &base(), &limits())).is_empty());
    }

    #[test]
    fn injected_regressions_fail() {
        let mut r = base();
        inject(&mut r, "relink_p95_ms=3").expect("inject");
        inject(&mut r, "pss_kb=1.5").expect("inject");
        assert_eq!(
            failed(&check(&r, &base(), &limits())),
            ["pss_kb", "relink_p95_ms"]
        );
        assert!(inject(&mut r, "nope=2").is_err());
        assert!(inject(&mut r, "pss_kb").is_err());
    }

    #[test]
    fn absolute_targets_hold_without_a_baseline() {
        let r = metrics(&[
            ("relink_p95_ms", 140.0),
            ("pss_kb", 4000.0),
            ("bit_exact", 0.0),
            ("added_latency_samples", 1024.0),
        ]);
        let v = check(&r, &Metrics::new(), &limits());
        assert_eq!(
            failed(&v),
            ["added_latency_samples", "bit_exact", "relink_p95_ms"]
        );
        assert!(v.iter().any(|x| x.why.contains("above the target 100")));
    }

    #[test]
    fn a_missing_measurement_fails() {
        let mut r = base();
        r.remove("pss_kb");
        let v = check(&r, &base(), &limits());
        assert_eq!(failed(&v), ["pss_kb"]);
        assert_eq!(
            v.iter().find(|x| !x.ok).map(|x| x.why.as_str()),
            Some("not measured")
        );
    }
}
