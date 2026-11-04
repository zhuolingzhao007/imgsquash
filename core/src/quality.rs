//! 测量与选择分离。感知分、全图分位误差和最差 8×8 块 RMSE 只描述候选。
//! ICC 没有在此转换，SSIMULACRA2 采用 sRGB 假设；不能据此承诺视觉无损。
use image::RgbaImage;
use ssimulacra2::{compute_frame_ssimulacra2, ColorPrimaries, Rgb, TransferCharacteristic};

/// 复核样本的区域坐标；尺寸至少覆盖当前输入可用的 128×128 范围。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct Metrics {
    pub score: f64,
    /// 全图 99.9 分位可见误差，仅作诊断；它会漏掉小面积损伤。
    pub err: f64,
    /// 最差 8×8 块的可见误差 RMSE，不允许小块损伤被整图平均稀释。
    pub local_error: f64,
    /// 同一误差扫描中记录平均可见误差最大的区域，供失败后小块复核。
    pub damage: Option<Region>,
}
impl Metrics {
    pub fn distortion(self) -> f64 {
        if !self.score.is_finite() {
            return f64::INFINITY;
        }
        (100.0 - self.score).max(0.0) / 100.0
    }
    pub const LOSSLESS: Self = Self {
        score: 100.0,
        err: 0.0,
        local_error: 0.0,
        damage: None,
    };
}

pub fn measure(orig: &RgbaImage, cand: &RgbaImage) -> Result<Metrics, String> {
    if orig.dimensions() != cand.dimensions() || orig.width() == 0 || orig.height() == 0 {
        return Err("候选尺寸不一致或图片为空".into());
    }
    let (err, local_error, damage) = visible_errors(orig, cand);
    if local_error == 0.0 {
        return Ok(Metrics::LOSSLESS);
    }
    // SSIMULACRA2 明确不支持宽/高小于 8 的图。不能把计算失败伪装成 0 分。
    let score = if has_meaningful_alpha(orig) || has_meaningful_alpha(cand) {
        score_over(orig, cand, 0)?.min(score_over(orig, cand, 255)?)
    } else {
        score_over(orig, cand, 255)?
    };
    if !score.is_finite() {
        return Err("感知测量结果无效".into());
    }
    Ok(Metrics {
        score,
        err,
        local_error,
        damage,
    })
}

fn score_over(orig: &RgbaImage, cand: &RgbaImage, background: u8) -> Result<f64, String> {
    compute_frame_ssimulacra2(to_rgb(orig, background), to_rgb(cand, background))
        .map_err(|error| format!("感知测量失败：{error:?}"))
}

fn to_rgb(img: &RgbaImage, background: u8) -> Rgb {
    let bg = background as f32 / 255.0;
    let data = img
        .pixels()
        .map(|pixel| {
            let a = pixel[3] as f32 / 255.0;
            let blend = |c: u8| c as f32 / 255.0 * a + bg * (1.0 - a);
            [blend(pixel[0]), blend(pixel[1]), blend(pixel[2])]
        })
        .collect();
    Rgb::new(
        data,
        img.width() as usize,
        img.height() as usize,
        TransferCharacteristic::SRGB,
        ColorPrimaries::BT709,
    )
    .expect("尺寸已校验")
}

pub fn has_meaningful_alpha(img: &RgbaImage) -> bool {
    img.pixels().any(|p| p[3] != 255)
}

/// 独立的大图验收结果：最低分是每个覆盖块的最差值，不是整图 SSIMULACRA2。
pub struct TiledMetrics {
    pub metrics: Metrics,
    pub tiles: u64,
    pub complete: bool,
}

/// 512px 窗口、128px 重叠；覆盖全部像素，8×8 网格对齐。读取器可以来自磁盘。
pub fn measure_tiled(
    width: u32,
    height: u32,
    mut read: impl FnMut(Region) -> Result<(RgbaImage, RgbaImage), String>,
    keep_scoring: impl Fn(Metrics) -> bool,
) -> Result<TiledMetrics, String> {
    if width < 8 || height < 8 {
        return Err("分块结构测量要求宽高至少 8 像素".into());
    }
    let mut result = Metrics::LOSSLESS;
    let mut tiles = 0;
    let mut y = 0;
    loop {
        let mut x = 0;
        loop {
            let region = Region {
                x,
                y,
                width: (width - x).min(512),
                height: (height - y).min(512),
            };
            let (a, b) = read(region)?;
            if a.dimensions() != (region.width, region.height) || b.dimensions() != a.dimensions() {
                return Err("分块读取器返回的尺寸不符".into());
            }
            let m = measure(&a, &b)?;
            result.score = result.score.min(m.score);
            result.err = result.err.max(m.err);
            result.local_error = result.local_error.max(m.local_error);
            tiles += 1;
            if !keep_scoring(result) {
                return Ok(TiledMetrics {
                    metrics: result,
                    tiles,
                    complete: false,
                });
            }
            if width - x <= 512 {
                break;
            }
            x += 384;
        }
        if height - y <= 512 {
            break;
        }
        y += 384;
    }
    Ok(TiledMetrics {
        metrics: result,
        tiles,
        complete: true,
    })
}

fn visible_errors(orig: &RgbaImage, cand: &RgbaImage) -> (f64, f64, Option<Region>) {
    let mut histogram = [0u64; 256];
    let columns = orig.width().div_ceil(128);
    let mut regions = vec![0u64; (columns * orig.height().div_ceil(128)) as usize];
    let blocks = orig.width().div_ceil(8) as usize;
    let mut squares = vec![0u64; blocks];
    let mut counts = vec![0u32; blocks];
    let mut worst: f64 = 0.0;
    for y in 0..orig.height() {
        for x in 0..orig.width() {
            let a = orig.get_pixel(x, y);
            let b = cand.get_pixel(x, y);
            let mut error = a[3].abs_diff(b[3]) as u32;
            for channel in 0..3 {
                // 黑、白背景的最大差包括任意纯色背景的端点；不可见 RGB 不参与。
                let pa = (a[channel] as u32 * a[3] as u32 + 127) / 255;
                let pb = (b[channel] as u32 * b[3] as u32 + 127) / 255;
                error = error
                    .max(pa.abs_diff(pb))
                    .max((pa + 255 - a[3] as u32).abs_diff(pb + 255 - b[3] as u32));
            }
            histogram[error as usize] += 1;
            if error != 0 {
                regions[(y / 128 * columns + x / 128) as usize] += (error * error) as u64;
            }
            squares[x as usize / 8] += (error * error) as u64;
            counts[x as usize / 8] += 1;
        }
        if y % 8 == 7 || y + 1 == orig.height() {
            for (sum, count) in squares.iter_mut().zip(&mut counts) {
                worst = worst.max((*sum as f64 / *count as f64).sqrt());
                *sum = 0;
                *count = 0;
            }
        }
    }
    let target = (orig.width() as u64 * orig.height() as u64 * 999).div_ceil(1000);
    let mut seen = 0;
    let percentile = histogram
        .iter()
        .position(|count| {
            seen += count;
            seen >= target
        })
        .unwrap_or(255);
    let damage = if worst == 0.0 {
        None
    } else {
        let mean = |index: usize, sum: u64| {
            let x = index as u32 % columns * 128;
            let y = index as u32 / columns * 128;
            let count = (orig.width() - x).min(128) as f64 * (orig.height() - y).min(128) as f64;
            sum as f64 / count
        };
        let (index, _) = regions
            .iter()
            .copied()
            .enumerate()
            .max_by(|(ia, a), (ib, b)| mean(*ia, *a).total_cmp(&mean(*ib, *b)))
            .expect("尺寸已校验");
        let width = orig.width().min(128);
        let height = orig.height().min(128);
        Some(Region {
            x: (index as u32 % columns * 128).min(orig.width() - width),
            y: (index as u32 / columns * 128).min(orig.height() - height),
            width,
            height,
        })
    };
    (percentile as f64, worst, damage)
}

/// 搜索阶段的轻量漏斗：先检查亮度与可见颜色 RMSE，再计算积分图 SSIM。
/// 缓冲区属于单张图的搜索会话，候选间复用；全图验收仍用 measure。
pub struct Funnel {
    reference: std::sync::Arc<RgbaImage>,
    integrals: Vec<[f64; 5]>,
}
impl Funnel {
    pub fn new(reference: std::sync::Arc<RgbaImage>) -> Self {
        Self {
            reference,
            integrals: Vec::new(),
        }
    }
    pub fn evaluate(
        &mut self,
        candidate: &RgbaImage,
        limits: crate::policy::Limits,
    ) -> Result<Option<Metrics>, String> {
        let reference = &self.reference;
        if reference.dimensions() != candidate.dimensions()
            || reference.width().min(reference.height()) < 8
        {
            return Err("采样图尺寸不一致或小于 8 像素".into());
        }
        let (w, h) = (reference.width() as usize, reference.height() as usize);
        let (mut luma_sq, mut visible_sq) = (0.0, 0.0);
        for (a, b) in reference.pixels().zip(candidate.pixels()) {
            let x = luma_on_white(a.0);
            let y = luma_on_white(b.0);
            luma_sq += (x - y) * (x - y);
            // 颜色/alpha 保护不能被亮度相同的换色骗过。
            let mut worst = a[3].abs_diff(b[3]) as f64;
            for c in 0..3 {
                let pa = a[c] as f64 * a[3] as f64 / 255.0;
                let pb = b[c] as f64 * b[3] as f64 / 255.0;
                worst = worst
                    .max((pa - pb).abs())
                    .max((pa + 255.0 - a[3] as f64 - pb - 255.0 + b[3] as f64).abs());
            }
            visible_sq += worst * worst;
        }
        let n = (w * h) as f64;
        if (luma_sq / n).sqrt() > limits.luma_rmse || (visible_sq / n).sqrt() > limits.visible_rmse
        {
            return Ok(None);
        }
        if visible_sq == 0.0 {
            return Ok(Some(Metrics::LOSSLESS));
        }
        let stride = w + 1;
        self.integrals.resize(stride * (h + 1), [0.0; 5]);
        self.integrals.fill([0.0; 5]);
        for y in 0..h {
            let mut row = [0.0; 5];
            for x in 0..w {
                let a = luma_on_white(reference.get_pixel(x as u32, y as u32).0);
                let b = luma_on_white(candidate.get_pixel(x as u32, y as u32).0);
                for (i, value) in [a, b, a * a, b * b, a * b].into_iter().enumerate() {
                    row[i] += value;
                    self.integrals[(y + 1) * stride + x + 1][i] =
                        row[i] + self.integrals[y * stride + x + 1][i];
                }
            }
        }
        let (mut sum, mut count) = (0.0, 0);
        for y in (0..=h - 8).step_by(4) {
            for x in (0..=w - 8).step_by(4) {
                let mut v = [0.0; 5];
                for (i, value) in v.iter_mut().enumerate() {
                    *value = (self.integrals[(y + 8) * stride + x + 8][i]
                        + self.integrals[y * stride + x][i]
                        - self.integrals[y * stride + x + 8][i]
                        - self.integrals[(y + 8) * stride + x][i])
                        / 64.0;
                }
                let va = (v[2] - v[0] * v[0]).max(0.0);
                let vb = (v[3] - v[1] * v[1]).max(0.0);
                let cov = v[4] - v[0] * v[1];
                sum += ((2.0 * v[0] * v[1] + 6.5025) * (2.0 * cov + 58.5225))
                    / ((v[0] * v[0] + v[1] * v[1] + 6.5025) * (va + vb + 58.5225));
                count += 1;
            }
        }
        if sum / (count as f64) < limits.ssim {
            return Ok(None);
        }
        // 普通 SSIM 对平滑渐变的色带不敏感。在小采样块上增加感知检查，
        // 仍比反复做全图检查便宜；预留 15% 失真余量抵抗采样偏差。
        let metrics = measure(reference, candidate)?;
        // 保留实际度量，包括超出最后结构/局部预算的点；搜索负责可行性，
        // 首次全图失败后仍可复用这些已测信息。前两级拒绝仍不追加感知计算。
        Ok(Some(metrics))
    }
}
fn luma_on_white(p: [u8; 4]) -> f64 {
    let y = (77.0 * p[0] as f64 + 150.0 * p[1] as f64 + 29.0 * p[2] as f64) / 256.0;
    y * p[3] as f64 / 255.0 + 255.0 - p[3] as f64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiled_validation_covers_edges_and_detects_small_damage_without_a_full_score() {
        let a = RgbaImage::from_pixel(1000, 777, image::Rgba([120, 120, 120, 255]));
        let mut b = a.clone();
        for y in 768..777 {
            for x in 992..1000 {
                b.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        let result = measure_tiled(
            1000,
            777,
            |r| {
                assert!(r.width <= 512 && r.height <= 512);
                Ok((
                    image::imageops::crop_imm(&a, r.x, r.y, r.width, r.height).to_image(),
                    image::imageops::crop_imm(&b, r.x, r.y, r.width, r.height).to_image(),
                ))
            },
            |_| true,
        )
        .unwrap();
        assert_eq!(result.tiles, 6);
        assert!(result.complete);
        assert_eq!(result.metrics.local_error, 120.0);
        assert!(result.metrics.score < 100.0);
    }
    #[test]
    fn tiled_validation_marks_early_rejection_incomplete() {
        let result = measure_tiled(
            1000,
            777,
            |r| {
                Ok((
                    RgbaImage::from_pixel(r.width, r.height, image::Rgba([255, 255, 255, 255])),
                    RgbaImage::from_pixel(r.width, r.height, image::Rgba([0, 0, 0, 255])),
                ))
            },
            |m| crate::policy::Preset::Strict.policy().allows(m),
        )
        .unwrap();
        assert_eq!(result.tiles, 1);
        assert!(!result.complete);
        assert!(!crate::policy::Preset::Strict
            .policy()
            .allows(result.metrics));
    }
    use image::Rgba;
    #[test]
    fn identical_and_invisible_rgb_are_lossless_even_on_tiny_images() {
        let a = RgbaImage::from_pixel(1, 1, Rgba([255, 0, 0, 0]));
        let b = RgbaImage::from_pixel(1, 1, Rgba([0, 255, 0, 0]));
        assert_eq!(measure(&a, &b).unwrap().score, 100.0);
        assert_eq!(measure(&a, &b).unwrap().local_error, 0.0);
    }
    #[test]
    fn small_visible_change_is_an_explicit_unsupported_measurement() {
        let a = RgbaImage::from_pixel(1, 1, Rgba([0, 0, 0, 255]));
        let b = RgbaImage::from_pixel(1, 1, Rgba([50, 0, 0, 255]));
        assert!(measure(&a, &b).is_err());
        assert!(measure(&a, &RgbaImage::new(2, 2)).is_err());
    }
    #[test]
    fn small_text_damage_cannot_hide_in_the_global_percentile() {
        let a = RgbaImage::from_pixel(256, 256, Rgba([255, 255, 255, 255]));
        let mut b = a.clone();
        for y in 0..4 {
            for x in 0..4 {
                b.put_pixel(x, y, Rgba([0, 0, 0, 255]));
            }
        }
        let metrics = measure(&a, &b).unwrap();
        assert_eq!(metrics.err, 0.0, "旧的分位门闸会漏掉这块损伤");
        assert!(metrics.local_error >= 127.0);
        assert_eq!(
            metrics.damage,
            Some(Region {
                x: 0,
                y: 0,
                width: 128,
                height: 128
            })
        );
        assert!(metrics.score < 100.0);
    }
    #[test]
    fn alpha_change_is_visible_on_white_even_when_black_hides_it() {
        let a = RgbaImage::from_pixel(32, 32, Rgba([0, 0, 0, 255]));
        let b = RgbaImage::from_pixel(32, 32, Rgba([0, 0, 0, 90]));
        assert_eq!(measure(&a, &b).unwrap().err, 165.0);
    }
    #[test]
    fn funnel_keeps_invisible_changes_and_rejects_equal_luma_recoloring() {
        use crate::policy::Preset;
        use std::sync::Arc;
        let a = RgbaImage::from_pixel(32, 32, Rgba([255, 0, 0, 0]));
        let b = RgbaImage::from_pixel(32, 32, Rgba([0, 255, 0, 0]));
        assert!(Funnel::new(Arc::new(a))
            .evaluate(&b, Preset::Standard.policy().limits())
            .unwrap()
            .is_some());
        let a = RgbaImage::from_pixel(32, 32, Rgba([255, 0, 0, 255]));
        let b = RgbaImage::from_pixel(32, 32, Rgba([0, 131, 0, 255]));
        assert!(
            (luma_on_white(a.get_pixel(0, 0).0) - luma_on_white(b.get_pixel(0, 0).0)).abs() < 1.0
        );
        assert!(Funnel::new(Arc::new(a))
            .evaluate(&b, Preset::Smaller.policy().limits())
            .unwrap()
            .is_none());
    }
    #[test]
    fn funnel_returns_existing_metrics_for_cost_ranking() {
        use crate::policy::Preset;
        use std::sync::Arc;
        let a = RgbaImage::from_pixel(256, 256, Rgba([128, 128, 128, 255]));
        let uniform = RgbaImage::from_pixel(256, 256, Rgba([129, 129, 129, 255]));
        let limits = Preset::Standard.policy().limits();
        let mut funnel = Funnel::new(Arc::new(a.clone()));
        let measured = funnel.evaluate(&uniform, limits).unwrap().unwrap();
        assert_eq!(measured.local_error, 1.0);
        assert!(measured.score.is_finite());
    }
    #[test]
    fn damage_region_is_clamped_to_valid_pixels_on_partial_edge_tiles() {
        let original = RgbaImage::from_pixel(257, 129, Rgba([255, 255, 255, 255]));
        let mut candidate = original.clone();
        candidate.put_pixel(256, 128, Rgba([0, 0, 0, 255]));
        let (_, _, region) = visible_errors(&original, &candidate);
        let region = region.unwrap();
        assert_eq!(region.width, 128);
        assert_eq!(region.height, 128);
        assert_eq!(region.x + region.width, 257);
        assert_eq!(region.y + region.height, 129);
        assert!(measure(&original, &original).unwrap().damage.is_none());
    }
}
