//! 档位策略：失真预算、采样余量、最低收益与率失真成本集中定义。
use crate::quality::Metrics;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Policy {
    pub quality_weight: f64,
}

impl Policy {
    /// 无损基线成本为 1；采样与最终结果共享同一成本定义。
    pub fn cost(self, bytes: usize, baseline: usize, metrics: Metrics) -> f64 {
        if bytes == 0 || baseline == 0 {
            return f64::INFINITY;
        }
        self.cost_ratio(bytes as f64 / baseline as f64, metrics)
    }
    pub fn cost_ratio(self, byte_ratio: f64, metrics: Metrics) -> f64 {
        if !byte_ratio.is_finite()
            || byte_ratio <= 0.0
            || !metrics.score.is_finite()
            || !metrics.local_error.is_finite()
            || !self.quality_weight.is_finite()
            || self.quality_weight < 0.0
        {
            return f64::INFINITY;
        }
        // 局部 RMSE 独立验收；D 只表示整体结构损失，避免两个预算混用。
        byte_ratio + self.quality_weight * metrics.distortion()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Preset {
    #[default]
    Standard,
    Strict,
    Smaller,
}

impl Preset {
    pub const ALL: [Self; 3] = [Self::Standard, Self::Strict, Self::Smaller];
    pub fn key(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Strict => "strict",
            Self::Smaller => "smaller",
        }
    }
    pub fn parse(value: &str) -> Result<Self, String> {
        Self::ALL
            .into_iter()
            .find(|preset| preset.key() == value)
            .ok_or_else(|| format!("未知的质量档位：{value}"))
    }
    pub fn policy(self) -> Policy {
        // 产品偏好，非经主观实验校准的阈值。策略与编码器无关。
        Policy {
            quality_weight: match self {
                Self::Standard => 2.0,
                Self::Strict => 5.0,
                Self::Smaller => 0.8,
            },
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub luma_rmse: f64,
    pub visible_rmse: f64,
    pub local_rmse: f64,
    pub ssim: f64,
    pub distortion: f64,
    pub min_gain: f64,
}
impl Limits {
    pub fn allows(self, metrics: Metrics) -> bool {
        metrics.score.is_finite()
            && metrics.local_error.is_finite()
            && metrics.local_error >= 0.0
            && metrics.local_error <= self.local_rmse
            && metrics.distortion() <= self.distortion
    }
    pub fn sample_distortion(self) -> f64 {
        self.distortion * 0.85
    }
}
impl Policy {
    pub fn limits(self) -> Limits {
        if self.quality_weight >= 4.0 {
            Limits {
                luma_rmse: 5.0,
                visible_rmse: 12.0,
                local_rmse: 24.0,
                ssim: 0.980,
                distortion: 0.12,
                min_gain: 0.03,
            }
        } else if self.quality_weight <= 1.0 {
            Limits {
                luma_rmse: 12.0,
                visible_rmse: 32.0,
                local_rmse: 64.0,
                ssim: 0.920,
                distortion: 0.32,
                min_gain: 0.01,
            }
        } else {
            Limits {
                luma_rmse: 8.0,
                visible_rmse: 22.0,
                local_rmse: 44.0,
                ssim: 0.950,
                distortion: 0.24,
                min_gain: 0.02,
            }
        }
    }
    pub fn allows(self, metrics: Metrics) -> bool {
        self.limits().allows(metrics)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn preferences_trade_real_savings_for_distortion() {
        let metrics = Metrics {
            score: 80.0,
            err: 24.0,
            local_error: 20.0,
            ..Metrics::LOSSLESS
        };
        let policy = Preset::Standard.policy();
        assert!(policy.cost(300, 1000, metrics) < 1.0);
        assert!(policy.cost(900, 1000, metrics) > 1.0);
        assert!(Preset::Strict.policy().cost(300, 1000, metrics) > 1.0);
    }
    #[test]
    fn every_preset_round_trips_and_orders_quality_cost() {
        for preset in Preset::ALL {
            assert_eq!(Preset::parse(preset.key()), Ok(preset));
        }
        assert!(Preset::Strict.policy().quality_weight > Preset::Standard.policy().quality_weight);
        assert!(Preset::Standard.policy().quality_weight > Preset::Smaller.policy().quality_weight);
        assert!(Preset::parse("unknown").is_err());
    }
    #[test]
    fn local_damage_has_its_own_limit_even_when_global_loss_is_small() {
        for preset in Preset::ALL {
            let policy = preset.policy();
            let limits = policy.limits();
            assert_eq!(limits.local_rmse, 2.0 * limits.visible_rmse);
            let metrics = Metrics {
                score: 99.0,
                local_error: limits.local_rmse,
                err: 0.0,
                ..Metrics::LOSSLESS
            };
            assert!(policy.allows(metrics));
            assert!(!policy.allows(Metrics {
                local_error: limits.local_rmse + 0.01,
                ..metrics
            }));
            assert!(!policy.allows(Metrics {
                local_error: f64::NAN,
                ..metrics
            }));
            // 全局失真相同意味着相同成本；局部伤害由独立约束拒绝。
            assert_eq!(
                policy.cost(100, 1000, metrics),
                policy.cost(
                    100,
                    1000,
                    Metrics {
                        local_error: limits.local_rmse + 1.0,
                        ..metrics
                    }
                )
            );
        }
    }
}
