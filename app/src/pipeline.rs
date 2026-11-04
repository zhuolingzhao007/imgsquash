//! 格式无关的最终候选验收。全图最多尝试两个参数，输出仍需满足真实收益。
use crate::squash::Candidate;
use tinypng_core::{policy::Policy, search::Plan};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Speed {
    Fast,
    Final,
}

/// PNG 可在编码前拒绝质量；保留实测值，不把正常质量拒绝伪装成编码错误。
pub(crate) trait Payload {
    fn byte_len(&self) -> usize;
}
impl Payload for Vec<u8> {
    fn byte_len(&self) -> usize {
        self.len()
    }
}

pub(crate) enum Trial<T = Vec<u8>> {
    Encoded(Candidate<T>),
    /// 已编码字节数不足以达到最低收益，无需再解码或测量质量。
    Unprofitable {
        bytes: usize,
    },
    Rejected {
        metrics: tinypng_core::quality::Metrics,
        refined: Option<Vec<(u32, tinypng_core::quality::Metrics)>>,
    },
}

pub(crate) fn finalists<T: Payload>(
    plan: Plan,
    baseline: usize,
    policy: Policy,
    mut encode: impl FnMut(u32) -> Result<Trial<T>, String>,
) -> Result<Option<Candidate<T>>, String> {
    let timing = crate::timing::Timing::new("acceptance");
    let mut best: Option<(f64, Candidate<T>)> = None;
    let mut first_error = None;
    let mut completed = false;
    let mut parameter = plan.primary;
    for index in 0..2 {
        let mut observation = None;
        let mut refinement = None;
        match encode(parameter) {
            Ok(Trial::Unprofitable { bytes }) => {
                completed = true;
                timing.run(
                    format!("parameter={parameter} bytes={bytes} savings_rejected_before_scoring"),
                    || (),
                );
            }
            Ok(Trial::Rejected { metrics, refined }) => {
                completed = true;
                observation = Some(metrics);
                refinement = refined;
                timing.run(
                    format!(
                        "parameter={parameter} D={:.8} quality_rejected_before_encoding",
                        metrics.distortion()
                    ),
                    || (),
                );
            }
            Ok(Trial::Encoded(candidate)) => {
                completed = true;
                observation = Some(candidate.metrics);
                let cost = policy.cost(candidate.bytes.byte_len(), baseline, candidate.metrics);
                let gain = if candidate.lossy {
                    policy.limits().min_gain
                } else {
                    0.0
                };
                let profitable =
                    crate::squash::is_profitable(baseline, candidate.bytes.byte_len(), gain);
                timing.run(format!("parameter={parameter} ratio={:.8} D={:.8} C={cost:.8} quality_ok={} savings_ok={}",
                    candidate.bytes.byte_len() as f64 / baseline as f64, candidate.metrics.distortion(),
                    policy.allows(candidate.metrics), profitable), || ());
                if policy.allows(candidate.metrics)
                    && cost < 1.0
                    && profitable
                    && best.as_ref().is_none_or(|(old, _)| cost < *old)
                {
                    best = Some((cost, candidate));
                }
            }
            Err(error) => {
                timing.run(
                    format!("parameter={parameter} no_encoded_result={error}"),
                    || (),
                );
                first_error.get_or_insert(error);
            }
        }
        if index == 1 || (best.is_some() && !plan.close) {
            break;
        }
        let next = if let Some(actual) = observation.filter(|m| !policy.allows(*m)) {
            for (q, prediction) in plan.predictions(actual, policy, refinement.as_deref()) {
                let metrics = prediction.metrics.expect("预测保留度量");
                timing.run(
                    format!(
                        "predict-parameter={q} D={:.8} local={:.6} C={:.8} feasible={}",
                        metrics.distortion(),
                        metrics.local_error,
                        prediction.cost,
                        prediction.cost.is_finite()
                    ),
                    || (),
                );
            }
            plan.corrected(actual, policy, refinement.as_deref())
        } else {
            // 编码故障、仅收益失败或近似平手，均没有质量校准的依据。
            Some(plan.backup)
        };
        let Some(next) = next.filter(|q| *q != plan.primary) else {
            break;
        };
        parameter = next;
    }
    if !completed {
        return Err(first_error.unwrap_or_else(|| "没有可用的全图结果".into()));
    }
    Ok(best.map(|(_, candidate)| candidate))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tinypng_core::{policy::Preset, quality::Metrics};
    fn candidate(bytes: usize, score: f64) -> Trial {
        Trial::Encoded(Candidate {
            bytes: vec![0; bytes],
            method: "test".into(),
            metrics: Metrics {
                score,
                ..Metrics::LOSSLESS
            },
            lossy: true,
        })
    }
    fn recorded_pair(primary: u32, backup: u32, close: bool) -> Plan {
        use tinypng_core::search::Probe;
        let policy = Preset::Standard.policy();
        let mut samples = vec![(
            primary,
            Probe::measured(400, 0.4, Some(metric(0.01, 1.0)), policy),
        )];
        if primary != backup {
            let (bytes, ratio, d) = if close {
                (408, 0.408, 0.01)
            } else {
                (600, 0.6, 0.001)
            };
            samples.push((
                backup,
                Probe::measured(bytes, ratio, Some(metric(d, 1.0)), policy),
            ));
        }
        Plan {
            primary,
            backup,
            close,
            samples,
        }
    }
    #[test]
    fn accepted_primary_avoids_a_second_full_encode() {
        let mut seen = Vec::new();
        let best = finalists::<Vec<u8>>(
            recorded_pair(40, 95, false),
            1000,
            Preset::Standard.policy(),
            |q| {
                seen.push(q);
                Ok(candidate(400, 95.0))
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(seen, [40]);
        assert_eq!(best.bytes.len(), 400);
    }
    #[test]
    fn insufficient_savings_falls_back_without_fabricating_quality_metrics() {
        let mut seen = Vec::new();
        let result = finalists::<Vec<u8>>(
            recorded_pair(40, 95, false),
            1000,
            Preset::Standard.policy(),
            |q| {
                seen.push(q);
                Ok(Trial::Unprofitable { bytes: 990 })
            },
        )
        .unwrap();
        assert!(result.is_none());
        assert_eq!(seen, [40, 95]);
    }
    #[test]
    fn failed_full_validation_tries_exactly_one_conservative_fallback() {
        let mut seen = Vec::new();
        let best = finalists::<Vec<u8>>(
            recorded_pair(40, 95, false),
            1000,
            Preset::Standard.policy(),
            |q| {
                seen.push(q);
                Ok(candidate(400, if q == 40 { 20.0 } else { 95.0 }))
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(seen, [40, 95]);
        assert_eq!(best.metrics.score, 95.0);
    }
    #[test]
    fn close_candidates_are_compared_after_strong_encoding() {
        let mut calls = 0;
        let best = finalists::<Vec<u8>>(
            recorded_pair(40, 50, true),
            1000,
            Preset::Standard.policy(),
            |q| {
                calls += 1;
                Ok(candidate(if q == 40 { 500 } else { 400 }, 95.0))
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(calls, 2);
        assert_eq!(best.bytes.len(), 400);
    }
    #[test]
    fn duplicate_parameters_and_encoder_failures_are_bounded() {
        let mut calls = 0;
        let result = finalists::<Vec<u8>>(
            recorded_pair(95, 95, false),
            1000,
            Preset::Standard.policy(),
            |_| {
                calls += 1;
                Err("encoder failed".into())
            },
        );
        assert!(result.is_err());
        assert_eq!(calls, 1);
    }
    fn metric(d: f64, local: f64) -> tinypng_core::quality::Metrics {
        tinypng_core::quality::Metrics {
            score: 100.0 * (1.0 - d),
            local_error: local,
            ..tinypng_core::quality::Metrics::LOSSLESS
        }
    }
    fn measured_plan() -> Plan {
        use tinypng_core::search::Probe;
        let policy = Preset::Standard.policy();
        Plan {
            primary: 10,
            backup: 20,
            close: false,
            samples: vec![
                (
                    10,
                    Probe::measured(100, 0.10, Some(metric(0.10, 5.0)), policy),
                ),
                (
                    20,
                    Probe::measured(50, 0.05, Some(metric(0.20, 4.0)), policy),
                ),
                (
                    30,
                    Probe::measured(350, 0.35, Some(metric(0.08, 6.0)), policy),
                ),
            ],
        }
    }
    #[test]
    fn measured_failure_reranks_without_a_third_full_attempt() {
        let mut seen = Vec::new();
        let output = finalists::<Vec<u8>>(measured_plan(), 1000, Preset::Standard.policy(), |q| {
            seen.push(q);
            if q == 10 {
                Ok(Trial::Rejected {
                    metrics: metric(0.25, 10.0),
                    refined: None,
                })
            } else {
                Ok(candidate(300, 95.0))
            }
        })
        .unwrap()
        .unwrap();
        assert_eq!(seen, [10, 30]);
        assert_eq!(output.bytes.len(), 300);
    }
    #[test]
    fn no_predicted_candidate_means_no_second_full_attempt() {
        let mut seen = Vec::new();
        let output = finalists::<Vec<u8>>(measured_plan(), 1000, Preset::Standard.policy(), |q| {
            seen.push(q);
            Ok(Trial::Rejected {
                metrics: metric(0.9, 100.0),
                refined: None,
            })
        })
        .unwrap();
        assert!(output.is_none());
        assert_eq!(seen, [10]);
    }
    #[test]
    fn savings_failure_keeps_the_original_runner_without_distortion_calibration() {
        use tinypng_core::search::Probe;
        let policy = Preset::Standard.policy();
        let plan = Plan {
            primary: 10,
            backup: 20,
            close: false,
            samples: vec![
                (
                    10,
                    Probe::measured(100, 0.10, Some(metric(0.20, 5.0)), policy),
                ),
                (
                    20,
                    Probe::measured(500, 0.50, Some(metric(0.02, 4.0)), policy),
                ),
                (
                    30,
                    Probe::measured(200, 0.20, Some(metric(0.19, 6.0)), policy),
                ),
            ],
        };
        let mut seen = Vec::new();
        finalists::<Vec<u8>>(plan, 1000, policy, |q| {
            seen.push(q);
            Ok(if q == 10 {
                candidate(990, 99.0)
            } else {
                candidate(300, 95.0)
            })
        })
        .unwrap()
        .unwrap();
        assert_eq!(seen, [10, 20]);
    }
}
