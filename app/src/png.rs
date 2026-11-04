//! PNG：单次画像，采样量化搜索，最终最多两个全图候选。
use crate::{
    convert::{self, Conversion},
    pipeline::{self, Speed},
    squash::{self, Candidate, Mode, Options, Outcome},
    timing::Timing,
};
use image::{DynamicImage, ImageDecoder, RgbaImage};
use tinypng_core::{
    analysis::{Analyzed, Content},
    png_meta,
    quality::{Funnel, Metrics, measure},
    quantize, search,
};

struct Info {
    bit_depth: u8,
    animated: bool,
}
pub fn squash(input: &[u8], options: &Options) -> Result<Outcome, String> {
    let timing = Timing::new("png");
    let info = inspect(input)?;
    if info.animated {
        return Ok(squash::unchanged(input, "original")
            .with_hint(squash::format_skipped(options, "动画 PNG 重编码会丢帧")));
    }
    if info.bit_depth == 16
        || options.mode == Mode::Lossless
        || (matches!(input[25], 0 | 4) && png_meta::has_iccp(input))
    {
        let reason = if info.bit_depth == 16 {
            "16 位 PNG 转格式会丢精度"
        } else if options.mode == Mode::Lossless {
            "无损模式保持原格式"
        } else {
            "灰度 ICC PNG 仅做无损优化"
        };
        return Ok(timing
            .run("lossless-final", || lossless(input, Speed::Final))
            .with_hint(squash::format_skipped(options, reason)));
    }
    if input.len() < 2048 && options.output_format == squash::OutputFormat::Keep {
        return Ok(timing
            .run("tiny/lossless-fast", || lossless(input, Speed::Fast))
            .with_hint(squash::format_skipped(
                options,
                "输入小于 2 KiB，优先保留原格式",
            )));
    }
    let (pixels, profile, orientation) = timing.run("decode", || {
        let mut decoder = image::codecs::png::PngDecoder::new(std::io::Cursor::new(input))
            .map_err(|e| e.to_string())?;
        let profile = decoder.icc_profile().map_err(|e| e.to_string())?;
        let orientation = decoder.orientation().map_err(|e| e.to_string())?;
        let pixels = DynamicImage::from_decoder(decoder)
            .map_err(|e| e.to_string())?
            .to_rgba8();
        Ok::<_, String>((pixels, profile, orientation))
    })?;
    let analyzed = timing.run("image-analysis", || Analyzed::new(pixels));
    timing.run(
        format!(
            "content/{:?} alpha={} gray={} colors={:?} flat={:.3} edges={:.3}",
            analyzed.profile.content,
            analyzed.profile.has_alpha,
            analyzed.profile.grayscale,
            analyzed.profile.unique_colors,
            analyzed.profile.flat_ratio,
            analyzed.profile.edge_density
        ),
        || (),
    );
    let baseline = timing.run("lossless-baseline-fast", || lossless(input, Speed::Fast));
    let mut hint = None;
    if options.output_format != squash::OutputFormat::Keep {
        let unmanaged = png_meta::has_chunk(input, b"cICP")
            || (!png_meta::has_iccp(input)
                && !png_meta::has_chunk(input, b"sRGB")
                && [b"gAMA", b"cHRM"]
                    .iter()
                    .any(|kind| png_meta::has_chunk(input, kind)));
        if unmanaged || (png_meta::has_iccp(input) && profile.is_none()) {
            hint = Some("未转换为 WebP：不能可靠传递源图的色彩描述".into());
        } else {
            let oriented;
            let source = if orientation == image::metadata::Orientation::NoTransforms {
                &analyzed
            } else {
                let mut image = DynamicImage::ImageRgba8((*analyzed.pixels).clone());
                image.apply_orientation(orientation);
                oriented = timing.run("oriented-analysis", || Analyzed::new(image.to_rgba8()));
                &oriented
            };
            match convert::attempt(source, &baseline.bytes, profile.as_deref(), options) {
                Conversion::Done(outcome) => return Ok(outcome),
                Conversion::Rejected(reason) => hint = Some(reason),
                Conversion::NotRequested => {}
            }
        }
    }
    let result = quantize_png(input, &analyzed, &baseline, options, &timing);
    Ok(match result {
        Ok(Some(candidate)) => squash::choose(baseline, candidate, options),
        Ok(None) => finish_lossless(baseline, &timing),
        Err(reason) => finish_lossless(baseline, &timing)
            .with_hint(Some(format!("量化候选未采用，保留无损结果：{reason}"))),
    }
    .with_hint(hint))
}
fn quantize_png(
    input: &[u8],
    image: &Analyzed,
    baseline: &Outcome,
    options: &Options,
    timing: &Timing,
) -> Result<Option<Candidate>, String> {
    let original = &image.pixels;
    if let Some(palette) = &image.profile.exact {
        return Ok(
            match final_palette(input, original, palette, options, timing)? {
                pipeline::Trial::Encoded(candidate) => Some(candidate),
                pipeline::Trial::Rejected { .. } | pipeline::Trial::Unprofitable { .. } => None,
            },
        );
    }
    if original.width().min(original.height()) < 8 {
        return Ok(None);
    }
    let dither = if image.profile.content == Content::Photo {
        quantize::Dither::Diffusion
    } else {
        quantize::Dither::None
    };
    // 照片仅保留 256 色尝试；本项目实图证明它可在标准档带来收益。
    let minimum = if image.profile.content == Content::Photo {
        256
    } else {
        16
    };
    let mut prepared = timing.run("histogram-full", || quantize::Prepared::new(original))?;
    // 代表性样本比全图更集中于细节，不能按全图平均 bpp 缩放无损基线。
    // 只建立一次同样本快速无损参照，不增加候选参数或全图编码。
    let sample_baseline = if minimum < 256 {
        Some(timing.run("sample/lossless-baseline-fast", || {
            let mut bytes = Vec::new();
            {
                let mut encoder =
                    ::png::Encoder::new(&mut bytes, image.sample.width(), image.sample.height());
                encoder.set_compression(::png::Compression::Fast);
                encoder.set_color(::png::ColorType::Rgba);
                encoder.set_depth(::png::BitDepth::Eight);
                let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
                writer
                    .write_image_data(image.sample.as_raw())
                    .map_err(|e| e.to_string())?;
                writer.finish().map_err(|e| e.to_string())?;
            }
            Ok::<_, String>(lossless(&bytes, Speed::Fast).bytes.len())
        })?)
    } else {
        // 照片只有一个颜色数，不需要为排名额外编码样本基线。
        None
    };
    let mut funnel = Funnel::new(std::sync::Arc::clone(&image.sample));
    let plan = search::bounded(minimum, 256, 5, |colors| {
        let palette = timing.run(format!("sample-c{colors}/quantize"), || {
            prepared.quantize_on(&image.sample, colors, dither)
        })?;
        let decoded = timing.run("sample/expand", || {
            palette.to_image(image.sample.width(), image.sample.height())
        });
        let metrics = timing.run("sample/funnel", || {
            funnel.evaluate(&decoded, options.policy.limits())
        })?;
        let Some(metrics) = metrics else {
            return Ok(search::Probe::UNMEASURED);
        };
        let bytes = timing.run("sample/encode-fast", || {
            palette.to_png(image.sample.width(), image.sample.height())
        })?;
        let ratio = match sample_baseline {
            Some(reference) => bytes.len() as f64 / reference as f64,
            None => image.sample_ratio(bytes.len(), baseline.bytes.len()),
        };
        let probe = search::Probe::measured(bytes.len(), ratio, Some(metrics), options.policy);
        timing.run(
            format!(
                "sample-c{colors}/ratio={ratio:.8} D={:.8} score={:.4} local={:.6} C={:.8}",
                metrics.distortion(),
                metrics.score,
                metrics.local_error,
                probe.cost
            ),
            || (),
        );
        Ok(probe)
    })?;
    let Some(plan) = plan else {
        return Ok(None);
    };
    let parameters: Vec<_> = plan
        .samples
        .iter()
        .filter(|(_, p)| p.ratio.is_finite())
        .map(|(q, _)| *q)
        .collect();
    let mut attempts = 0;
    pipeline::finalists(plan, baseline.bytes.len(), options.policy, |colors| {
        attempts += 1;
        let palette = timing.run(format!("final-c{colors}/quantize"), || {
            prepared.quantize(colors, dither)
        })?;
        let mut trial = final_palette(input, original, &palette, options, timing)?;
        if attempts == 1
            && parameters.len() > 1
            && matches!(dither, quantize::Dither::None)
            && let pipeline::Trial::Rejected { metrics, refined } = &mut trial
        {
            *refined = refine_palette_samples(
                &mut prepared,
                original,
                &parameters,
                colors,
                *metrics,
                timing,
            );
        }
        Ok(trial)
    })
}
fn final_palette(
    input: &[u8],
    original: &RgbaImage,
    palette: &quantize::Palette,
    options: &Options,
    timing: &Timing,
) -> Result<pipeline::Trial, String> {
    let decoded = palette.to_image(original.width(), original.height());
    let metrics = timing.run("final/verify-full", || measure(original, &decoded))?;
    timing.run(
        format!(
            "final-c{}/D={:.8} limit={:.8} local={:.6} local_limit={:.6} quality_ok={}",
            palette.colors(),
            metrics.distortion(),
            options.policy.limits().distortion,
            metrics.local_error,
            options.policy.limits().local_rmse,
            options.policy.allows(metrics)
        ),
        || (),
    );
    if !options.policy.allows(metrics) {
        return Ok(pipeline::Trial::Rejected {
            metrics,
            refined: None,
        });
    }
    let bytes = timing.run("final/encode-fast", || {
        palette.to_png(original.width(), original.height())
    })?;
    let optimized = timing.run("final/optimize-strong", || optimize(&bytes, Speed::Final))?;
    Ok(pipeline::Trial::Encoded(Candidate {
        bytes: png_meta::copy_preserved(input, &optimized),
        method: format!("quantizr-c{}", palette.colors()),
        metrics,
        lossy: decoded != *original,
    }))
}
fn refine_palette_samples(
    prepared: &mut quantize::Prepared<'_>,
    original: &RgbaImage,
    parameters: &[u32],
    first: u32,
    actual: Metrics,
    timing: &Timing,
) -> Option<Vec<(u32, Metrics)>> {
    let region = actual.damage?;
    let tile = image::imageops::crop_imm(original, region.x, region.y, region.width, region.height)
        .to_image();
    let mut measurements = Vec::new();
    for parameter in parameters {
        let result = timing.run(format!("refine-c{parameter}/region-measure"), || {
            let palette = prepared.quantize_on(&tile, *parameter, quantize::Dither::None)?;
            measure(&tile, &palette.to_image(tile.width(), tile.height()))
        });
        match result {
            Ok(metrics) => {
                timing.run(
                    format!(
                        "refine-c{parameter}/x={} y={} D={:.8} local={:.6}",
                        region.x,
                        region.y,
                        metrics.distortion(),
                        metrics.local_error
                    ),
                    || (),
                );
                measurements.push((*parameter, metrics));
            }
            Err(error) => {
                timing.run(format!("refine-c{parameter}/unavailable={error}"), || ());
            }
        }
    }
    let reference = measurements.iter().find(|(q, _)| *q == first)?.1;
    let k_d = (reference.distortion() > 0.0).then(|| actual.distortion() / reference.distortion());
    let k_local = (reference.local_error > 0.0).then(|| actual.local_error / reference.local_error);
    timing.run(format!("calibration/full_D={:.8} region_D={:.8} k_D={k_d:?} k_local={k_local:?} zero_reference=additive",
        actual.distortion(), reference.distortion()), || ());
    Some(measurements)
}
fn finish_lossless(baseline: Outcome, timing: &Timing) -> Outcome {
    let stronger = timing.run("fallback/lossless-final", || {
        lossless(&baseline.bytes, Speed::Final)
    });
    if stronger.bytes.len() < baseline.bytes.len() {
        stronger
    } else {
        baseline
    }
}
fn lossless(input: &[u8], speed: Speed) -> Outcome {
    match optimize(input, speed) {
        Ok(bytes) if bytes.len() < input.len() => Outcome {
            kept_icc: squash::carries_icc(&bytes),
            bytes,
            extension: None,
            method: "oxipng".into(),
            score: Metrics::LOSSLESS.score,
            err: 0.0,
            lossy: false,
            hint: None,
        },
        Ok(_) => squash::unchanged(input, "kept"),
        Err(reason) => squash::unchanged(input, "kept").with_hint(Some(reason)),
    }
}
fn optimize(bytes: &[u8], speed: Speed) -> Result<Vec<u8>, String> {
    let mut options = oxipng::Options::from_preset(if speed == Speed::Fast { 0 } else { 2 });
    // iCCP 的色彩空间必须与颜色类型匹配（类型 0/4 要求 GRAY 配置）。oxipng 灰度降级
    // 不校验这一点，会把 RGB 输出降成灰度却原样保留 RGB 配置，libpng 系解码器将忽略
    // 该配置。带 iCCP 时禁用灰度降级，其余降级（降调色板等）不受影响。
    options.grayscale_reduction = !png_meta::has_iccp(bytes);
    oxipng::optimize_from_memory(bytes, &options).map_err(|e| format!("oxipng 失败：{e}"))
}
fn inspect(bytes: &[u8]) -> Result<Info, String> {
    if bytes.len() < 33 || !bytes.starts_with(squash::PNG_SIGNATURE) || &bytes[12..16] != b"IHDR" {
        return Err("不是有效的 PNG".into());
    }
    Ok(Info {
        bit_depth: bytes[24],
        animated: png_meta::has_chunk(bytes, b"acTL"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{ImageFormat, Rgba, RgbaImage};

    fn icon() -> RgbaImage {
        RgbaImage::from_fn(256, 256, |x, y| {
            let dx = x as i32 - 128;
            let dy = y as i32 - 128;
            let distance = dx * dx + dy * dy;
            if distance < 90 * 90 {
                Rgba([30, 120, 220, 255])
            } else if distance < 104 * 104 {
                Rgba([30, 120, 220, 90])
            } else {
                Rgba([0, 0, 0, 0])
            }
        })
    }

    fn png_bytes(image: &RgbaImage) -> Vec<u8> {
        let mut bytes = Vec::new();
        image
            .write_to(&mut std::io::Cursor::new(&mut bytes), ImageFormat::Png)
            .unwrap();
        bytes
    }

    fn png16_bytes() -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, 32, 32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Sixteen);
            let mut writer = encoder.write_header().unwrap();
            let data: Vec<u8> = (0..32 * 32 * 8).map(|index| (index % 255) as u8).collect();
            writer.write_image_data(&data).unwrap();
            writer.finish().unwrap();
        }
        out
    }

    #[test]
    fn smart_png_quantizes_and_shrinks() {
        let original = icon();
        let input = png_bytes(&original);
        let outcome = squash(&input, &Options::default()).unwrap();
        assert!(!outcome.lossy, "精确调色板无需有损量化");
        assert!(outcome.bytes.len() < input.len());
        assert_eq!(outcome.score, 100.0);
        assert_eq!(outcome.extension, None, "保持原格式时不改扩展名");
        let decoded = image::load_from_memory(&outcome.bytes).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), original.dimensions());
    }

    #[test]
    fn strict_ui_stays_lossy_within_ten_percent_of_the_previous_result() {
        let input = include_bytes!("../design/layout-1180.png");
        let options = Options {
            policy: tinypng_core::policy::Preset::Strict.policy(),
            ..Options::default()
        };
        let outcome = squash(input, &options).unwrap();
        assert!(
            outcome.lossy,
            "严格档 UI 不应悄悄退回无损：{} B，{:?}",
            outcome.bytes.len(),
            outcome.hint
        );
        assert!(
            outcome.bytes.len() <= 88_613 * 11 / 10,
            "输出 {} B 超过原有损结果 88,613 B 的 110%",
            outcome.bytes.len()
        );
        assert!(outcome.score >= 88.0);
    }

    #[test]
    fn icc_png_quantizes_and_keeps_profile() {
        // 带 ICC 的图也要走量化（原来会整体退回无损，只能压 7% 左右），
        // 量化后原 iCCP 必须贴回输出，颜色语义才不变。
        let input = std::fs::read("tests/fixtures/icon-icc.png").unwrap();
        assert!(png_meta::has_iccp(&input), "fixture 应带 iCCP");
        let outcome = squash(&input, &Options::default()).unwrap();
        assert!(!outcome.lossy, "带 ICC 的精确调色板仍然无损");
        assert!(outcome.kept_icc, "输出应保留 ICC");
        assert!(outcome.bytes.len() < input.len());
        assert_eq!(
            png_meta::chunk_before_idat(&outcome.bytes, b"iCCP"),
            png_meta::chunk_before_idat(&input, b"iCCP"),
            "量化后应贴回原 iCCP"
        );
        let decoded = image::load_from_memory(&outcome.bytes).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (256, 256));
    }

    #[test]
    fn gray_pixels_with_iccp_are_not_downgraded_to_grayscale() {
        // oxipng 会把全灰 RGB(A) 降成灰度，但原 iCCP 是 RGB 色彩空间。
        // 降级后 profile 与颜色类型失配，输出违反 PNG 规范，libpng 系解码器会忽略该配置。
        let fixture = std::fs::read("tests/fixtures/icon-icc.png").unwrap();
        for (color_type, expected) in [(png::ColorType::Rgb, 2u8), (png::ColorType::Rgba, 6u8)] {
            let channels = if color_type == png::ColorType::Rgb {
                3
            } else {
                4
            };
            let mut raw = Vec::with_capacity(64 * 64 * channels);
            for _ in 0..64 * 64 {
                raw.extend_from_slice(&[77, 77, 77]);
                if channels == 4 {
                    raw.push(255);
                }
            }
            let mut bytes = Vec::new();
            {
                let mut encoder = png::Encoder::new(&mut bytes, 64, 64);
                encoder.set_color(color_type);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().unwrap();
                writer.write_image_data(&raw).unwrap();
                writer.finish().unwrap();
            }
            let input = png_meta::copy_preserved(&fixture, &bytes);
            assert_eq!(input[25], expected, "前置条件：输入应为颜色类型 {expected}");
            let options = Options {
                mode: Mode::Lossless,
                ..Options::default()
            };
            let outcome = squash(&input, &options).unwrap();
            // 合规要求是「颜色类型不得降为 0/4 灰度」：类型 2/3/6 都允许配 RGB profile。
            // 降调色板（2→3）是无损且合规的，不该被误伤。
            let output_type = outcome.bytes[25];
            assert!(
                !matches!(output_type, 0 | 4),
                "全灰像素 + iCCP 不允许灰度降级（输出颜色类型 {output_type}），否则 iCCP 色彩空间失配"
            );
            assert!(outcome.kept_icc, "输出应保留 iCCP");
        }
    }

    #[test]
    fn png_to_webp_uses_decompressed_icc_bytes() {
        let fixture = std::fs::read("tests/fixtures/icon-icc.png").unwrap();
        // ICC 在 PNG 中压缩而在 WebP 中不压缩；小图标转换不一定有收益。
        // 用实际有转换空间的纹理图验证 ICC 传递，而非要求输出更大的 WebP。
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let pixels = RgbaImage::from_fn(256, 256, |x, y| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let noise = ((seed >> 33) % 17) as i32 - 8;
            let c = |v: u32| (v as i32 + noise).clamp(0, 255) as u8;
            Rgba([c(x), c(y), c((x + y) / 2), 255])
        });
        let input = png_meta::copy_preserved(&fixture, &png_bytes(&pixels));
        let mut decoder =
            image::codecs::png::PngDecoder::new(std::io::Cursor::new(&input)).unwrap();
        let expected = decoder.icc_profile().unwrap().unwrap();
        let options = Options {
            output_format: squash::OutputFormat::WebP,
            ..Options::default()
        };
        let output = squash(&input, &options).unwrap();
        assert_eq!(output.extension.as_deref(), Some("webp"));
        assert_eq!(
            webpx::get_icc_profile(&output.bytes).unwrap().as_deref(),
            Some(expected.as_slice())
        );
        assert!(png_meta::chunk_before_idat(&input, b"iCCP").unwrap() != expected.as_slice());
    }

    #[test]
    fn webp_compares_against_the_optimized_png_including_icc_overhead() {
        let input = std::fs::read("tests/fixtures/icon-icc.png").unwrap();
        let baseline = lossless(&input, Speed::Fast);
        let output = squash(
            &input,
            &Options {
                output_format: squash::OutputFormat::WebP,
                ..Options::default()
            },
        )
        .unwrap();
        assert_eq!(output.extension, None);
        assert!(output.bytes.len() <= baseline.bytes.len());
        assert!(output.kept_icc);
        assert!(output.hint.unwrap().contains("未转换为 WebP"));
        assert_eq!(
            squash::decode(&input, ImageFormat::Png).unwrap(),
            squash::decode(&output.bytes, ImageFormat::Png).unwrap()
        );
    }

    #[test]
    fn lossless_keeps_every_pixel() {
        let original = icon();
        let input = png_bytes(&original);
        let options = Options {
            mode: Mode::Lossless,
            ..Options::default()
        };
        let outcome = squash(&input, &options).unwrap();
        assert!(!outcome.lossy);
        assert_eq!(outcome.score, 100.0);
        let decoded = image::load_from_memory(&outcome.bytes).unwrap().to_rgba8();
        assert_eq!(decoded, original);
    }

    #[test]
    fn png16_falls_back_to_lossless_with_reason() {
        let input = png16_bytes();
        let outcome = squash(&input, &Options::default()).unwrap();
        assert!(["oxipng", "kept"].contains(&outcome.method.as_str()));
        assert!(!outcome.lossy);
        assert!(outcome.bytes.len() <= input.len());
    }

    #[test]
    fn apng_is_left_alone() {
        let mut bytes = squash::PNG_SIGNATURE.to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&8u32.to_be_bytes());
        ihdr.extend_from_slice(&8u32.to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes.extend_from_slice(&(ihdr.len() as u32).to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&ihdr);
        bytes.extend_from_slice(&[0, 0, 0, 0]);
        bytes.extend_from_slice(&8u32.to_be_bytes());
        bytes.extend_from_slice(b"acTL");
        bytes.extend_from_slice(&[0, 0, 0, 2, 0, 0, 0, 0]);
        bytes.extend_from_slice(&[0, 0, 0, 0]);

        let outcome = squash(&bytes, &Options::default()).unwrap();
        assert_eq!(outcome.method, "original");
        assert_eq!(outcome.bytes, bytes);
    }

    #[test]
    fn quantized_pixels_match_encoded_png() {
        // SSIMULACRA2 直接按调色板展开来算，等价于「编码成索引 PNG 再解码」。这条测试
        // 守住这个等价关系，避免以后为了省事改坏判定。
        let opaque = RgbaImage::from_fn(64, 64, |x, y| {
            Rgba([(x * 4) as u8, (y * 4) as u8, ((x + y) * 2) as u8, 255])
        });
        for original in [icon(), opaque] {
            let mut prepared = quantize::Prepared::new(&original).unwrap();
            for colors in [16, 128, quantize::MAX_COLORS] {
                let palette = prepared.quantize(colors, quantize::Dither::None).unwrap();
                let expanded = palette.to_image(original.width(), original.height());
                let encoded = palette.to_png(original.width(), original.height()).unwrap();
                let decoded = image::load_from_memory_with_format(&encoded, ImageFormat::Png)
                    .unwrap()
                    .to_rgba8();
                assert_eq!(expanded, decoded, "c{colors} 展开结果应与解码一致");
            }
        }
    }

    #[test]
    fn lossless_mode_explains_why_it_skips_conversion() {
        let input = png_bytes(&icon());
        let options = Options {
            mode: Mode::Lossless,
            output_format: squash::OutputFormat::WebP,
            ..Options::default()
        };
        let outcome = squash(&input, &options).unwrap();
        assert_eq!(outcome.extension, None, "无损模式不转格式");
        let hint = outcome.hint.unwrap();
        assert!(hint.contains("WebP"), "{hint}");
        assert!(hint.contains("无损"), "{hint}");
    }
}
