//! 采样质量驱动的有界二分。单调性仅是搜索启发式，全图验证与次优参数负责回退。
use crate::{policy::Policy, quality::Metrics};

#[derive(Debug, Clone, Copy)]
pub struct Probe {
    pub bytes: usize,
    /// 不可行点使用正无穷；有限成本参与二分可行性和最终排名。
    pub cost: f64,
    pub ratio: f64,
    pub metrics: Option<Metrics>,
}
impl Probe {
    pub const UNMEASURED: Self = Self {
        bytes: 0,
        cost: f64::INFINITY,
        ratio: f64::INFINITY,
        metrics: None,
    };
    pub fn measured(bytes: usize, ratio: f64, metrics: Option<Metrics>, policy: Policy) -> Self {
        let cost = metrics
            .filter(|m| policy.allows(*m) && m.distortion() <= policy.limits().sample_distortion())
            .map_or(f64::INFINITY, |m| policy.cost_ratio(ratio, m));
        Self {
            bytes,
            ratio,
            metrics,
            cost,
        }
    }
}
#[derive(Debug, Clone)]
pub struct Plan {
    pub primary: u32,
    pub backup: u32,
    pub close: bool,
    /// 包括已有度量但原先未通过预算的点，不包含编码/测量失败的点。
    pub samples: Vec<(u32, Probe)>,
}
impl Plan {
    /// 首次质量失败后重新预测全部已探测候选；refined 必须使用同一复核区域。
    pub fn predictions(
        &self,
        actual: Metrics,
        policy: Policy,
        refined: Option<&[(u32, Metrics)]>,
    ) -> Vec<(u32, Probe)> {
        let sample = |parameter| match refined {
            Some(values) => values
                .iter()
                .find(|(q, _)| *q == parameter)
                .map(|(_, m)| *m),
            None => self
                .samples
                .iter()
                .find(|(q, _)| *q == parameter)
                .and_then(|(_, p)| p.metrics),
        };
        let Some(reference) = sample(self.primary) else {
            return Vec::new();
        };
        self.samples
            .iter()
            .filter_map(|(parameter, probe)| {
                if *parameter == self.primary {
                    return None;
                }
                let measured = sample(*parameter)?;
                let prediction = Metrics {
                    score: 100.0
                        * (1.0
                            - transfer(
                                reference.distortion(),
                                actual.distortion(),
                                measured.distortion(),
                            )),
                    local_error: transfer(
                        reference.local_error,
                        actual.local_error,
                        measured.local_error,
                    ),
                    ..measured
                };
                let cost = if policy.allows(prediction) {
                    policy.cost_ratio(probe.ratio, prediction)
                } else {
                    f64::INFINITY
                };
                Some((
                    *parameter,
                    Probe {
                        cost,
                        metrics: Some(prediction),
                        ..*probe
                    },
                ))
            })
            .collect()
    }
    pub fn corrected(
        &self,
        actual: Metrics,
        policy: Policy,
        refined: Option<&[(u32, Metrics)]>,
    ) -> Option<u32> {
        self.predictions(actual, policy, refined)
            .into_iter()
            .filter(|(_, p)| p.cost.is_finite())
            .min_by(|(qa, a), (qb, b)| {
                a.cost
                    .total_cmp(&b.cost)
                    .then(a.bytes.cmp(&b.bytes))
                    .then(qa.cmp(qb))
            })
            .map(|(q, _)| q)
    }
}
fn transfer(reference: f64, actual: f64, value: f64) -> f64 {
    if reference > 0.0 {
        value * (actual / reference)
    } else {
        value + actual
    } // 零样本误差无法求比例，保守使用已观察到的绝对偏差。
}

pub fn bounded(
    low: u32,
    high: u32,
    budget: usize,
    mut probe: impl FnMut(u32) -> Result<Probe, String>,
) -> Result<Option<Plan>, String> {
    if low > high || budget == 0 {
        return Err("搜索范围或预算无效".into());
    }
    let top = probe(high)?;
    if !top.cost.is_finite() {
        return Ok(None);
    }
    let mut samples = vec![(high, top)];
    let (mut left, mut right) = (low, high);
    for _ in 1..budget {
        if left >= right {
            break;
        }
        let middle = left + (right - left) / 2;
        match probe(middle) {
            Ok(candidate) => {
                let feasible = candidate.cost.is_finite();
                samples.push((middle, candidate));
                if feasible {
                    right = middle;
                } else {
                    left = middle + 1;
                }
            }
            Err(_) => left = middle + 1,
        }
    }
    // 预算只决定可行性；档位成本决定首选和次选，不能只按体积排序。
    samples.sort_by(|(qa, a), (qb, b)| {
        a.cost
            .total_cmp(&b.cost)
            .then(a.bytes.cmp(&b.bytes))
            .then(qa.cmp(qb))
    });
    let mut accepted = samples.iter().filter(|(_, p)| p.cost.is_finite());
    let primary = *accepted.next().expect("最高质量端已通过");
    let runner = accepted.next().copied();
    let close = runner.is_some_and(|runner| {
        primary.1.bytes.abs_diff(runner.1.bytes) as f64
            <= primary.1.bytes.min(runner.1.bytes) as f64 * 0.02
    });
    // 失败时回退次优已测可行点；只有一个可行点时不重复编码。
    let backup = runner.unwrap_or(primary);
    Ok(Some(Plan {
        primary: primary.0,
        backup: backup.0,
        close,
        samples,
    }))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn finds_boundary_without_duplicate_or_unbounded_probes() {
        let mut seen = std::collections::BTreeSet::new();
        let plan = bounded(0, 100, 5, |q| {
            assert!(seen.insert(q));
            Ok(Probe {
                cost: if q >= 70 { q as f64 } else { f64::INFINITY },
                bytes: q as usize + 100,
                ..Probe::UNMEASURED
            })
        })
        .unwrap()
        .unwrap();
        assert!(plan.primary >= 70 && plan.primary <= 76);
        assert!(plan.backup >= plan.primary);
        assert!(seen.len() <= 5);
    }
    #[test]
    fn a_failed_probe_keeps_the_known_safe_point() {
        let plan = bounded(0, 100, 5, |q| {
            if q == 100 {
                Ok(Probe {
                    cost: 100.0,
                    bytes: 100,
                    ..Probe::UNMEASURED
                })
            } else {
                Err("broken".into())
            }
        })
        .unwrap()
        .unwrap();
        assert_eq!(plan.primary, 100);
        assert!(bounded(0, 100, 5, |_| Ok(Probe {
            cost: f64::INFINITY,
            bytes: 1,
            ..Probe::UNMEASURED
        }))
        .unwrap()
        .is_none());
    }
    #[test]
    fn close_runners_use_actual_size_even_when_parameter_order_reverses() {
        let plan = bounded(0, 100, 5, |q| {
            Ok(Probe {
                cost: match q {
                    25 => 100.0,
                    12 => 101.0,
                    _ => 200.0,
                },
                bytes: match q {
                    25 => 100,
                    12 => 101,
                    _ => 200,
                },
                ..Probe::UNMEASURED
            })
        })
        .unwrap()
        .unwrap();
        assert_eq!((plan.primary, plan.backup), (25, 12));
        assert!(plan.close);
    }
    #[test]
    fn fallback_uses_the_runner_instead_of_jumping_to_the_largest_output() {
        let plan = bounded(0, 100, 5, |q| {
            Ok(Probe {
                cost: if q >= 60 { q as f64 } else { f64::INFINITY },
                bytes: q as usize + 100,
                ..Probe::UNMEASURED
            })
        })
        .unwrap()
        .unwrap();
        assert_eq!((plan.primary, plan.backup), (63, 75));
        assert!(!plan.close);
    }
    #[test]
    fn weights_select_different_points_without_changing_the_probe_budget() {
        use crate::{policy::Preset, quality::Metrics};
        let mut traces = Vec::new();
        let mut winners = Vec::new();
        for preset in [Preset::Strict, Preset::Standard, Preset::Smaller] {
            let mut trace = Vec::new();
            let plan = bounded(0, 100, 5, |q| {
                trace.push(q);
                let (bytes, loss) = match q {
                    100 => (80, 0.0),
                    50 => (70, 0.03),
                    25 => (65, 0.08),
                    12 => (64, 0.09),
                    _ => (63, 0.10),
                };
                let metrics = Metrics {
                    score: 100.0 * (1.0 - loss),
                    ..Metrics::LOSSLESS
                };
                assert!(preset.policy().allows(metrics));
                Ok(Probe {
                    bytes,
                    cost: preset.policy().cost(bytes, 100, metrics),
                    ratio: bytes as f64 / 100.0,
                    metrics: Some(metrics),
                })
            })
            .unwrap()
            .unwrap();
            traces.push(trace);
            winners.push(plan.primary);
        }
        assert_eq!(winners, [100, 50, 6]);
        assert_eq!(traces[0], [100, 50, 25, 12, 6]);
        assert_eq!(traces[0], traces[1]);
        assert_eq!(traces[1], traces[2]);
    }
    fn metric(d: f64, local: f64) -> Metrics {
        Metrics {
            score: 100.0 * (1.0 - d),
            local_error: local,
            ..Metrics::LOSSLESS
        }
    }
    fn recorded(policy: Policy) -> Plan {
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
    fn calibration_can_select_a_point_beyond_the_original_runner() {
        let policy = crate::policy::Preset::Standard.policy();
        let plan = recorded(policy);
        assert_eq!(plan.corrected(metric(0.25, 10.0), policy, None), Some(30));
        let predictions = plan.predictions(metric(0.25, 10.0), policy, None);
        let third = predictions
            .iter()
            .find(|(q, _)| *q == 30)
            .unwrap()
            .1
            .metrics
            .unwrap();
        assert!((third.distortion() - 0.20).abs() < 1e-9);
        assert!((third.local_error - 12.0).abs() < 1e-9);
    }
    #[test]
    fn zero_sample_errors_use_observed_absolute_bias_without_nan() {
        let policy = crate::policy::Preset::Standard.policy();
        let mut plan = recorded(policy);
        plan.samples[0].1.metrics = Some(Metrics::LOSSLESS);
        assert_eq!(plan.corrected(metric(0.25, 10.0), policy, None), None);
        assert!(plan
            .predictions(metric(0.25, 10.0), policy, None)
            .iter()
            .all(|(_, p)| p.metrics.unwrap().distortion().is_finite()));
        assert_eq!(transfer(0.0, 0.0, 0.1), 0.1);
    }
    #[test]
    fn local_bias_is_calibrated_separately_from_global_loss() {
        let policy = crate::policy::Preset::Standard.policy();
        let mut plan = recorded(policy);
        plan.samples[1].1.metrics = Some(metric(0.05, 10.0));
        plan.samples[2].1.metrics = Some(metric(0.08, 3.0));
        assert_eq!(plan.corrected(metric(0.12, 50.0), policy, None), Some(30));
    }
    #[test]
    fn refinement_uses_the_same_region_for_reference_and_other_candidates() {
        let policy = crate::policy::Preset::Strict.policy();
        let plan = Plan {
            primary: 10,
            backup: 20,
            close: false,
            samples: vec![
                (
                    10,
                    Probe::measured(100, 0.1, Some(metric(0.07, 5.0)), policy),
                ),
                (
                    20,
                    Probe::measured(300, 0.3, Some(metric(0.04, 9.0)), policy),
                ),
                (
                    30,
                    Probe::measured(400, 0.4, Some(metric(0.06, 4.0)), policy),
                ),
            ],
        };
        let actual = metric(0.125, 10.0);
        assert_eq!(plan.corrected(actual, policy, None), Some(20));
        let refined = [
            (10, metric(0.25, 5.0)),
            (20, metric(0.50, 9.0)),
            (30, metric(0.20, 4.0)),
        ];
        assert_eq!(plan.corrected(actual, policy, Some(&refined)), Some(30));
        // 不混用原样本和复核区域的 D；缺少该区域度量的候选不参与这次预测。
        assert_eq!(plan.corrected(actual, policy, Some(&refined[..2])), None);
    }
}
