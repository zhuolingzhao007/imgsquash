//! 大图流程：有界工作集、磁盘产物与全覆盖分块验收。
use crate::{
    file_plan::Header,
    pipeline::{self, Payload, Trial},
    raster::Raster,
    squash::{Candidate, FileOutcome, Options, OutputFormat},
    stream_jpeg, stream_png,
    timing::Timing,
};
use std::{
    fs::File,
    io::{Seek, Write},
    path::Path,
};
use tinypng_core::{
    analysis::{Content, Profile},
    quality::{self, Funnel, Metrics},
    quantize::{Dither, Prepared},
    search,
};

pub(crate) struct Artifact {
    pub file: tempfile::NamedTempFile,
    size: usize,
}
impl Artifact {
    pub fn new(target: &Path) -> Result<Self, String> {
        let parent = target
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        Ok(Self {
            file: tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?,
            size: 0,
        })
    }
    pub fn refresh(&mut self) -> Result<(), String> {
        self.file.flush().map_err(|e| e.to_string())?;
        self.size = usize::try_from(
            self.file
                .as_file()
                .metadata()
                .map_err(|e| e.to_string())?
                .len(),
        )
        .map_err(|_| "编码结果超过平台地址范围")?;
        Ok(())
    }
}
impl Payload for Artifact {
    fn byte_len(&self) -> usize {
        self.size
    }
}

pub(crate) fn original(
    source: &mut File,
    header: &Header,
    length: u64,
    target: &Path,
    reason: String,
) -> Result<FileOutcome, String> {
    let mut output = Artifact::new(target)?;
    source.rewind().map_err(|e| e.to_string())?;
    if std::io::copy(&mut *source, output.file.as_file_mut())
        .map_err(|e| format!("复制原文件失败: {e}"))?
        != length
    {
        return Err("图片在处理过程中发生变化".into());
    }
    output.refresh()?;
    finish(
        Candidate {
            bytes: output,
            method: "original".into(),
            metrics: Metrics::LOSSLESS,
            lossy: false,
        },
        header,
        source,
        target,
        "global",
        Some(reason),
    )
}

pub(crate) fn run(
    source: &mut File,
    header: &Header,
    length: u64,
    target: &Path,
    options: &Options,
    lossy: bool,
) -> Result<FileOutcome, String> {
    let timing = Timing::new("streaming");
    let baseline = if let Some(png) = &header.png {
        let mut native = timing.run("decode-native-rows", || stream_png::decode(source, false))?;
        Some(timing.run("lossless-rows", || {
            stream_png::encode(&mut native, png, target, false)
        })?)
    } else {
        None
    };
    let baseline_size = baseline
        .as_ref()
        .map_or(length as usize, |b| (length as usize).min(b.byte_len()));
    let mut hint = if options.output_format == OutputFormat::WebP {
        Some("大图采用低内存处理，保持原格式和原尺寸，未转换 WebP".into())
    } else {
        Some("已使用低内存处理，保持原尺寸".into())
    };
    if lossy {
        let candidate = timing.run("lossy-stream", || {
            let mut original = timing.run("decode-rgba-rows", || {
                if header.png.is_some() {
                    stream_png::decode(source, true)
                } else {
                    stream_jpeg::decode(source)
                }
            })?;
            let sample = original.sample()?;
            let sample_pixels = sample.width() as f64 * sample.height() as f64;
            let pixels = original.width as f64 * original.height as f64;
            let mut funnel = Funnel::new(std::sync::Arc::new(sample.clone()));
            if let Some(png) = &header.png {
                let histogram = original.histogram_sample()?;
                let mut prepared = Prepared::new(&histogram)?;
                let profile = Profile::scan(&sample);
                let minimum = if profile.content == Content::Photo
                    || Profile::scan(&histogram).unique_colors.is_some()
                {
                    256
                } else {
                    16
                };
                let mut sample_bytes = Vec::new();
                image::DynamicImage::ImageRgba8(sample.clone())
                    .write_to(
                        &mut std::io::Cursor::new(&mut sample_bytes),
                        image::ImageFormat::Png,
                    )
                    .map_err(|e| e.to_string())?;
                let plan = search::bounded(minimum, 256, 5, |colors| {
                    let palette = prepared.quantize_on(&sample, colors, Dither::None)?;
                    let metrics = funnel.evaluate(
                        &palette.to_image(sample.width(), sample.height()),
                        options.policy.limits(),
                    )?;
                    let bytes = palette.to_png(sample.width(), sample.height())?;
                    Ok(search::Probe::measured(
                        bytes.len(),
                        bytes.len() as f64 / sample_bytes.len() as f64,
                        metrics,
                        options.policy,
                    ))
                })?;
                let Some(plan) = plan else {
                    return Ok(None);
                };
                pipeline::finalists(plan, baseline_size, options.policy, |colors| {
                    let (mut indexed, mut decoded, changed) =
                        timing.run("map-rows", || map_png(&mut original, &mut prepared, colors))?;
                    let measured = if changed {
                        timing.run("validate-all-tiles", || {
                            validate(&mut original, &mut decoded, options.policy)
                        })?
                    } else {
                        Metrics::LOSSLESS
                    };
                    if !options.policy.allows(measured) {
                        return Ok(Trial::Rejected {
                            metrics: measured,
                            refined: None,
                        });
                    }
                    let bytes = timing.run("encode-indexed-rows", || {
                        stream_png::encode(&mut indexed, png, target, true)
                    })?;
                    Ok(Trial::Encoded(Candidate {
                        bytes,
                        method: format!("stream-png-c{colors}"),
                        metrics: measured,
                        lossy: changed,
                    }))
                })
            } else {
                if Profile::scan(&sample).content == Content::Graphic {
                    return Ok(None);
                }
                let jpeg = header.jpeg.as_ref().ok_or("缺少 JPEG 图片头")?;
                let marker_bytes: usize = jpeg.markers.iter().map(|(_, data)| data.len() + 4).sum();
                let plan = search::bounded(
                    35,
                    u32::from(jpeg.info.quality.unwrap_or(95).min(95)),
                    5,
                    |q| {
                        let bytes = timing.run(format!("sample-q{q}/encode"), || {
                            stream_jpeg::encode_sample(&sample, jpeg, q as u8)
                        })?;
                        let decoded = crate::squash::decode(&bytes, image::ImageFormat::Jpeg)?;
                        let metrics = timing.run("sample/funnel", || {
                            funnel.evaluate(&decoded, options.policy.limits())
                        })?;
                        // ICC/EXIF 等是每文件固定开销，不能按样本与全图像素比放大。
                        let estimate = bytes.len().saturating_sub(marker_bytes) as f64 * pixels
                            / sample_pixels
                            + marker_bytes as f64;
                        Ok(search::Probe::measured(
                            bytes.len(),
                            estimate / length as f64,
                            metrics,
                            options.policy,
                        ))
                    },
                )?;
                let Some(plan) = plan else {
                    return Ok(None);
                };
                pipeline::finalists(plan, baseline_size, options.policy, |q| {
                    let mut bytes = timing.run("encode-jpeg-rows", || {
                        stream_jpeg::encode(&mut original, jpeg, q as u8, target)
                    })?;
                    if !crate::squash::is_profitable(
                        baseline_size,
                        bytes.byte_len(),
                        options.policy.limits().min_gain,
                    ) {
                        return Ok(Trial::Unprofitable {
                            bytes: bytes.byte_len(),
                        });
                    }
                    let mut decoded = timing.run("decode-candidate-rows", || {
                        stream_jpeg::decode(bytes.file.as_file_mut())
                    })?;
                    let metrics = timing.run("validate-all-tiles", || {
                        validate(&mut original, &mut decoded, options.policy)
                    })?;
                    Ok(Trial::Encoded(Candidate {
                        bytes,
                        method: format!("stream-jpeg-q{q}"),
                        metrics,
                        lossy: true,
                    }))
                })
            }
        });
        match candidate {
            Ok(Some(candidate)) => {
                return finish(candidate, header, source, target, "tiled", hint);
            }
            Ok(None) => {}
            Err(error) => hint = Some(format!("低内存有损处理未采用，保留无损或原文件：{error}")),
        }
    }
    if let Some(baseline) = baseline.filter(|b| b.byte_len() < length as usize) {
        finish(
            Candidate {
                bytes: baseline,
                method: "stream-png-lossless".into(),
                metrics: Metrics::LOSSLESS,
                lossy: false,
            },
            header,
            source,
            target,
            "global",
            hint,
        )
    } else {
        original(
            source,
            header,
            length,
            target,
            hint.unwrap_or_else(|| "保持原文件".into()),
        )
    }
}

fn map_png(
    original: &mut Raster,
    prepared: &mut Prepared<'_>,
    colors: u32,
) -> Result<(Raster, Raster, bool), String> {
    let first = original.region(quality::Region {
        x: 0,
        y: 0,
        width: original.width,
        height: 1,
    })?;
    let palette = prepared.quantize_on(&first, colors, Dither::None)?;
    let depth = match palette.colors() {
        0..=2 => png::BitDepth::One,
        3..=4 => png::BitDepth::Two,
        5..=16 => png::BitDepth::Four,
        _ => png::BitDepth::Eight,
    };
    let mut indexed = Raster::new(
        original.width,
        original.height,
        png::ColorType::Indexed,
        depth,
    )?;
    indexed.palette = Some(palette.rgb.iter().flatten().copied().collect());
    indexed.trns = (!palette.alpha.is_empty()).then(|| palette.alpha.clone());
    let bits = depth as usize;
    let mut packed = vec![0u8; indexed.stride()];
    let mut decoded = Raster::new(
        original.width,
        original.height,
        png::ColorType::Rgba,
        png::BitDepth::Eight,
    )?;
    let mut changed = false;
    let mut row = vec![0; original.stride()];
    for y in 0..original.height {
        original.row(y, &mut row)?;
        let image =
            image::RgbaImage::from_raw(original.width, 1, row.clone()).ok_or("映射行尺寸无效")?;
        let mapped = prepared.quantize_on(&image, colors, Dither::None)?;
        packed.fill(0);
        for (x, index) in mapped.indices.iter().enumerate() {
            packed[x * bits / 8] |= *index << (8 - bits - x * bits % 8);
        }
        indexed.write_row(y, &packed)?;
        let mapped_rgba = mapped.to_image(original.width, 1);
        changed |= mapped_rgba.as_raw() != &row;
        decoded.write_row(y, mapped_rgba.as_raw())?;
    }
    Ok((indexed, decoded, changed))
}
fn validate(
    a: &mut Raster,
    b: &mut Raster,
    policy: tinypng_core::policy::Policy,
) -> Result<Metrics, String> {
    let result = quality::measure_tiled(
        a.width,
        a.height,
        |r| Ok((a.region(r)?, b.region(r)?)),
        |m| policy.allows(m),
    )?;
    if !result.complete && policy.allows(result.metrics) {
        return Err("分块验证未完成".into());
    }
    Ok(result.metrics)
}

fn finish(
    candidate: Candidate<Artifact>,
    header: &Header,
    source: &File,
    target: &Path,
    score_kind: &str,
    hint: Option<String>,
) -> Result<FileOutcome, String> {
    let Candidate {
        bytes: mut artifact,
        method,
        metrics,
        lossy,
    } = candidate;
    let actual = Header::read(artifact.file.as_file_mut())?;
    if (actual.width, actual.height, actual.has_icc())
        != (header.width, header.height, header.has_icc())
    {
        return Err("流式输出尺寸或色彩配置校验失败".into());
    }
    if let (Some(a), Some(b)) = (&header.jpeg, &actual.jpeg)
        && (a.info.icc != b.info.icc || a.markers != b.markers)
    {
        return Err("JPEG 色彩/方向元数据回读不一致".into());
    }
    if let (Some(a), Some(b)) = (&header.png, &actual.png) {
        for kind in tinypng_core::png_meta::PRESERVED {
            if a.metadata.iter().find(|(k, _)| k == kind)
                != b.metadata.iter().find(|(k, _)| k == kind)
            {
                return Err("PNG 色彩/方向元数据回读不一致".into());
            }
        }
    }
    header.unchanged(source)?;
    artifact.refresh()?;
    let size = artifact.byte_len() as u64;
    if size > header.source_len {
        return Err("流式输出意外增大".into());
    }
    let path = crate::squash::publish(artifact.file, target)?;
    Ok(FileOutcome {
        input_size: header.source_len,
        output_size: size,
        output_path: path.to_string_lossy().into_owned(),
        method,
        score: metrics.score,
        err: metrics.err,
        lossy,
        kept_icc: actual.has_icc(),
        hint,
        score_kind: score_kind.into(),
    })
}
