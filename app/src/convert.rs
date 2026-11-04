//! WebP：照片跳过无损编码；图形跳过有损 DCT；其余在原尺寸采样块搜索。
use crate::{
    pipeline::{self, Speed},
    squash::{self, Candidate, Mode, Options, Outcome, OutputFormat},
    timing::Timing,
};
use image::{ImageFormat, RgbaImage};
use tinypng_core::{
    analysis::{Analyzed, Content},
    quality::{Funnel, Metrics, measure},
    search,
};
pub enum Conversion {
    NotRequested,
    Done(Outcome),
    Rejected(String),
}
pub fn attempt(
    image: &Analyzed,
    source: &[u8],
    profile: Option<&[u8]>,
    options: &Options,
) -> Conversion {
    if options.output_format == OutputFormat::Keep {
        return Conversion::NotRequested;
    }
    if options.mode == Mode::Lossless {
        return Conversion::Rejected("未转换为 WebP：无损模式保持原格式".into());
    }
    let timing = Timing::new("webp");
    let candidate = match encode(image, profile, source.len(), options, &timing) {
        Ok(candidate) => candidate,
        Err(reason) => return Conversion::Rejected(format!("未转换为 WebP：{reason}")),
    };
    let gain = if candidate.lossy {
        options.policy.limits().min_gain
    } else {
        0.0
    };
    if !squash::is_profitable(source.len(), candidate.bytes.len(), gain)
        || options
            .policy
            .cost(candidate.bytes.len(), source.len(), candidate.metrics)
            >= 1.0
    {
        return Conversion::Rejected("未转换为 WebP：收益不足以抵偿画质损失，保留原格式".into());
    }
    Conversion::Done(Outcome {
        kept_icc: squash::carries_icc(&candidate.bytes),
        bytes: candidate.bytes,
        extension: Some("webp".into()),
        method: candidate.method,
        score: candidate.metrics.score,
        err: candidate.metrics.err,
        lossy: candidate.lossy,
        hint: None,
    })
}
fn encode(
    image: &Analyzed,
    profile: Option<&[u8]>,
    source_size: usize,
    options: &Options,
    timing: &Timing,
) -> Result<Candidate, String> {
    let mut lossless = if image.profile.content == Content::Photo {
        None
    } else {
        Some(timing.run("lossless-baseline-fast", || {
            lossless_candidate(&image.pixels, profile, Speed::Fast, timing)
        })?)
    };
    let baseline = lossless.as_ref().map_or(source_size, |candidate| {
        source_size.min(candidate.bytes.len())
    });
    let lossless_only = matches!(image.profile.content, Content::Exact | Content::Graphic)
        || image.pixels.width().min(image.pixels.height()) < 8;
    let lossy = if lossless_only {
        None
    } else {
        let mut funnel = Funnel::new(std::sync::Arc::clone(&image.sample));
        let mut cache = std::collections::BTreeMap::new();
        let overhead = profile.map_or(0, |p| p.len() + 26);
        let mut probe = |quality: u32| -> Result<search::Probe, String> {
            if let Some(cached) = cache.get(&quality) {
                return Ok(*cached);
            }
            let bytes = timing.run(format!("sample-q{quality}/encode-fast"), || {
                encode_webp(&image.sample, None, false, quality as u8, Speed::Fast)
            })?;
            let decoded = timing.run("sample/decode", || {
                squash::decode(&bytes, ImageFormat::WebP)
            })?;
            let metrics = timing.run("sample/funnel", || {
                funnel.evaluate(&decoded, options.policy.limits())
            })?;
            let projection = search::Probe::measured(
                bytes.len(),
                image.sample_ratio(bytes.len(), baseline),
                metrics,
                options.policy,
            );
            cache.insert(quality, projection);
            Ok(projection)
        };
        // 中间质量的 bpp 只用于早期放弃，不把采样大小当作最终输出大小。
        let (low, high) = (20, 95);
        // 与二分的首个中点一致，预估编码不会增加一次搜索编码。
        let first = probe(low + (high - low) / 2)?;
        let estimate = first.bytes.saturating_sub(32) as f64 * image.pixels.len() as f64
            / image.sample.len().max(1) as f64
            + 32.0
            + overhead as f64;
        let plan = if estimate >= baseline as f64 * (1.0 - options.policy.limits().min_gain) {
            None
        } else {
            search::bounded(low, high, 5, &mut probe)?
        };
        if let Some(plan) = plan {
            pipeline::finalists(plan, baseline, options.policy, |quality| {
                let bytes = timing.run(format!("final-q{quality}/encode-strong"), || {
                    encode_webp(&image.pixels, profile, false, quality as u8, Speed::Final)
                })?;
                let decoded =
                    timing.run("final/decode", || squash::decode(&bytes, ImageFormat::WebP))?;
                let metrics =
                    timing.run("final/verify-full", || measure(&image.pixels, &decoded))?;
                timing.run(
                    format!(
                        "verified score={:.2} local={:.2}",
                        metrics.score, metrics.local_error
                    ),
                    || (),
                );
                Ok(pipeline::Trial::Encoded(Candidate {
                    bytes,
                    method: format!("webp-q{quality}"),
                    metrics,
                    lossy: decoded != *image.pixels,
                }))
            })?
        } else {
            None
        }
    };
    if let Some(candidate) = lossy {
        return Ok(candidate);
    }
    if let Some(fast) = lossless.take() {
        let strong = timing.run("lossless-final-strong", || {
            lossless_candidate(&image.pixels, profile, Speed::Final, timing)
        })?;
        return Ok(if strong.bytes.len() < fast.bytes.len() {
            strong
        } else {
            fast
        });
    }
    Err("采样或全图验证未找到合适的有损结果，照片跳过无损 WebP".into())
}
fn lossless_candidate(
    rgba: &RgbaImage,
    profile: Option<&[u8]>,
    speed: Speed,
    timing: &Timing,
) -> Result<Candidate, String> {
    let bytes = encode_webp(rgba, profile, true, 75, speed)?;
    let decoded = timing.run("lossless/verify-decode", || {
        squash::decode(&bytes, ImageFormat::WebP)
    })?;
    if decoded != *rgba {
        return Err("WebP 无损回读像素不一致".into());
    }
    Ok(Candidate {
        bytes,
        method: "webp".into(),
        metrics: Metrics::LOSSLESS,
        lossy: false,
    })
}
fn encode_webp(
    rgba: &RgbaImage,
    profile: Option<&[u8]>,
    lossless: bool,
    quality: u8,
    speed: Speed,
) -> Result<Vec<u8>, String> {
    let mut encoder = webpx::Encoder::new_rgba(rgba.as_raw(), rgba.width(), rgba.height())
        .lossless(lossless)
        .quality(quality as f32)
        .method(if speed == Speed::Fast { 0 } else { 6 })
        .exact(true)
        .alpha_quality(100);
    if let Some(profile) = profile {
        encoder = encoder.icc_profile(profile);
    }
    let bytes = encoder
        .encode(webpx::Unstoppable)
        .map_err(|e| format!("WebP 编码失败：{e}"))?;
    if let Some(profile) = profile {
        let actual = webpx::get_icc_profile(&bytes).map_err(|e| format!("ICC 回读失败：{e}"))?;
        if actual.as_deref() != Some(profile) {
            return Err("WebP 输出色彩配置不一致".into());
        }
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn photo(size: u32) -> RgbaImage {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        RgbaImage::from_fn(size, size, |x, y| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let noise = ((seed >> 33) % 17) as i32 - 8;
            let c = |v: u32| ((v * 255 / size) as i32 + noise).clamp(0, 255) as u8;
            image::Rgba([c(x), c(y), c((x + y) / 2), 255])
        })
    }
    #[test]
    fn photographic_content_benefits_from_lossy_encoding() {
        let rgba = photo(256);
        let lossless = encode_webp(&rgba, None, true, 75, Speed::Final).unwrap();
        let encoded = encode(
            &Analyzed::new(rgba.clone()),
            None,
            // 使用真实无损基准字节数；RGBA 内存长度不是压缩文件体积。
            lossless.len(),
            &Options::default(),
            &Timing::new("webp-test"),
        )
        .unwrap();
        assert!(encoded.lossy);
        assert!(encoded.bytes.len() * 2 < lossless.len());
        assert!(encoded.metrics.score.is_finite());
    }
    #[test]
    fn lossless_wins_on_flat_content() {
        let rgba = RgbaImage::from_pixel(256, 256, image::Rgba([20, 30, 40, 255]));
        let encoded = encode(
            &Analyzed::new(rgba.clone()),
            None,
            rgba.len(),
            &Options::default(),
            &Timing::new("webp-test"),
        )
        .unwrap();
        assert!(!encoded.lossy);
        assert_eq!(encoded.metrics.score, 100.0);
    }
    #[test]
    fn icc_and_all_alpha_values_round_trip_in_lossless_output() {
        let rgba = RgbaImage::from_fn(32, 32, |x, y| {
            image::Rgba([x as u8, y as u8, 100, (x * 8) as u8])
        });
        let profile = b"icc-payload-for-byte-identity-check";
        let bytes = encode_webp(&rgba, Some(profile), true, 75, Speed::Final).unwrap();
        assert_eq!(
            webpx::get_icc_profile(&bytes).unwrap().as_deref(),
            Some(profile.as_slice())
        );
        assert_eq!(squash::decode(&bytes, ImageFormat::WebP).unwrap(), rgba);
    }
    #[test]
    fn keep_and_lossless_modes_do_not_convert() {
        let rgba = photo(16);
        assert!(matches!(
            attempt(&Analyzed::new(rgba.clone()), &[], None, &Options::default()),
            Conversion::NotRequested
        ));
        let options = Options {
            mode: Mode::Lossless,
            output_format: OutputFormat::WebP,
            ..Options::default()
        };
        assert!(matches!(
            attempt(&Analyzed::new(rgba.clone()), &[], None, &options),
            Conversion::Rejected(_)
        ));
    }
}
