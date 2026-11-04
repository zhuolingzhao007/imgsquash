//! JPEG 分支：解析语义、原图解码、按实际率失真成本细化质量参数。
//!
//! 标记解析只走一遍：SOF 拿分量数和色度抽样，APP2 拿 ICC 分段。之前是两个几乎
//! 一样的循环各扫一遍，改一处忘一处。

use crate::convert::{self, Conversion};
use crate::squash::{self, Candidate, Mode, Options, Outcome};
use crate::{
    pipeline::{self, Speed},
    timing::Timing,
};
use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbaImage};
use std::io::Cursor;
use tinypng_core::{
    analysis::{Analyzed, Content},
    quality::{Funnel, measure},
    search,
};

/// 一帧 JPEG 里我们关心的东西。
pub struct Info {
    /// 分量数。4 表示 CMYK / YCCK。
    pub components: u8,
    /// 三个分量的 (h, v) 抽样因子，按 SOF 顺序。非 3 分量时为 None。
    pub sampling: Option<[(u8, u8); 3]>,
    /// 合并好的 ICC 配置。分段缺失时为 None，但 `has_icc` 仍是 true。
    pub icc: Option<Vec<u8>>,
    /// 文件里出现过 ICC_PROFILE 标记。
    pub has_icc: bool,
    /// 仅识别严格匹配 IJG 标准缩放表的质量；自定义表为 None。
    pub quality: Option<u8>,
}

pub fn squash(input: &[u8], options: &Options) -> Result<Outcome, String> {
    let timing = Timing::new("jpeg");
    let mut info = inspect(input);

    // CMYK / YCCK：mozjpeg 这里只吃 RGB，硬转会把颜色搞错。
    if info.components == 4 {
        return Ok(squash::unchanged(input, "original")
            .with_hint(squash::format_skipped(options, "CMYK JPEG 不重编码")));
    }
    // 灰度 ICC 不能直接附到 RGB 重编码或 WebP 上；未做颜色空间变换时保留原图。
    if info.components == 1 && info.has_icc {
        return Ok(squash::unchanged(input, "original").with_hint(Some(
            "灰度 ICC JPEG 保持原样，避免把灰度配置附到 RGB 输出".into(),
        )));
    }
    // ICC 分段不全：重编码会丢掉一半配置，比不压更糟。
    if info.has_icc && info.icc.is_none() {
        return Ok(squash::unchanged(input, "original")
            .with_hint(Some("ICC 配置分段不完整，为避免偏色保持原样".to_string())));
    }
    if options.mode == Mode::Lossless {
        return Ok(squash::unchanged(input, "original")
            .with_hint(squash::format_skipped(options, "无损模式不重编码 JPEG")));
    }

    if input.len() < 2048 || info.quality.is_some_and(|q| q <= 75) {
        return Ok(
            squash::unchanged(input, "original").with_hint(squash::format_skipped(
                options,
                "JPEG 已是低质量或极小文件，避免重复损失",
            )),
        );
    }
    let (original, swap_axes) = timing.run("decode", || decode_oriented(input))?;
    if swap_axes && let Some(sampling) = &mut info.sampling {
        for (h, v) in sampling {
            std::mem::swap(h, v);
        }
    }
    let image = timing.run("image-analysis", || Analyzed::new(original));
    timing.run(
        format!(
            "content/{:?} quality={:?} flat={:.3} edges={:.3}",
            image.profile.content,
            info.quality,
            image.profile.flat_ratio,
            image.profile.edge_density
        ),
        || (),
    );
    let mut hint = None;
    match convert::attempt(&image, input, info.icc.as_deref(), options) {
        Conversion::Done(outcome) => return Ok(outcome),
        Conversion::Rejected(reason) => hint = Some(reason),
        Conversion::NotRequested => {}
    }
    let baseline = squash::unchanged(input, "original");
    if image.profile.content == Content::Graphic
        || image.pixels.width().min(image.pixels.height()) < 8
    {
        return Ok(baseline.with_hint(hint));
    }
    let mut funnel = Funnel::new(std::sync::Arc::clone(&image.sample));
    let mut buffer = Vec::new();
    let maximum = info.quality.unwrap_or(95).min(95) as u32;
    let plan = search::bounded(35, maximum, 5, |quality| {
        let bytes = timing.run(format!("sample-q{quality}/encode-fast"), || {
            encode(
                &image.sample,
                quality as u8,
                &info,
                Speed::Fast,
                &mut buffer,
                false,
            )
        })?;
        let decoded = timing.run("sample/decode", || {
            squash::decode(&bytes, ImageFormat::Jpeg)
        })?;
        let metrics = timing.run("sample/funnel", || {
            funnel.evaluate(&decoded, options.policy.limits())
        })?;
        Ok(search::Probe::measured(
            bytes.len(),
            image.sample_ratio(bytes.len(), input.len()),
            metrics,
            options.policy,
        ))
    });
    let best = match plan {
        Ok(Some(plan)) => pipeline::finalists(plan, input.len(), options.policy, |quality| {
            let bytes = timing.run(format!("final-q{quality}/encode-strong"), || {
                encode(
                    &image.pixels,
                    quality as u8,
                    &info,
                    Speed::Final,
                    &mut buffer,
                    true,
                )
            })?;
            let decoded =
                timing.run("final/decode", || squash::decode(&bytes, ImageFormat::Jpeg))?;
            let metrics = timing.run("final/verify-full", || measure(&image.pixels, &decoded))?;
            timing.run(
                format!(
                    "verified score={:.2} local={:.2}",
                    metrics.score, metrics.local_error
                ),
                || (),
            );
            Ok(pipeline::Trial::Encoded(Candidate {
                bytes,
                method: format!("mozjpeg-q{quality}"),
                metrics,
                lossy: true,
            }))
        }),
        Ok(None) => Ok(None),
        Err(reason) => Err(reason),
    };
    Ok(match best {
        Ok(Some(candidate)) => squash::choose(baseline, candidate, options).with_hint(hint),
        Ok(None) => baseline.with_hint(hint),
        Err(reason) => baseline.with_hint(Some(format!("JPEG 重编码不可用，保留原图：{reason}"))),
    })
}
fn encode(
    original: &RgbaImage,
    quality: u8,
    info: &Info,
    speed: Speed,
    buffer: &mut Vec<u8>,
    metadata: bool,
) -> Result<Vec<u8>, String> {
    let mut compress = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
    compress.set_size(original.width() as usize, original.height() as usize);
    compress.set_quality(quality as f32);
    if speed == Speed::Final {
        compress.set_progressive_mode();
        compress.set_optimize_scans(true);
    } else {
        compress.set_optimize_scans(false);
    }
    if let Some(sampling) = info.sampling {
        compress.set_chroma_sampling_pixel_sizes(
            chroma_pixel_size(sampling[0], sampling[1]),
            chroma_pixel_size(sampling[0], sampling[2]),
        );
    }
    buffer.clear();
    buffer.reserve(original.width() as usize * original.height() as usize * 3);
    buffer.extend(original.pixels().flat_map(|p| [p[0], p[1], p[2]]));
    let mut started = compress
        .start_compress(Vec::new())
        .map_err(|e| format!("JPEG 编码失败：{e}"))?;
    started
        .write_scanlines(buffer)
        .map_err(|e| format!("JPEG 写入失败：{e}"))?;
    let mut bytes = started
        .finish()
        .map_err(|e| format!("JPEG 收尾失败：{e}"))?;
    if metadata && let Some(profile) = &info.icc {
        inject_icc(&mut bytes, profile);
    }
    Ok(bytes)
}

/// 色度相对亮度的像素尺寸：`(1,1)` 是 4:4:4，`(2,1)` 是 4:2:2，`(2,2)` 是 4:2:0。
pub(crate) fn chroma_pixel_size(luma: (u8, u8), chroma: (u8, u8)) -> (u8, u8) {
    let horizontal = (luma.0 / chroma.0.max(1)).max(1);
    let vertical = (luma.1 / chroma.1.max(1)).max(1);
    (horizontal, vertical)
}

/// 一趟扫完所有标记，产出 (marker, 负载)。
///
/// 停在 SOS 之前：熵编码数据里会出现假的 0xFF 序列，继续扫没有意义。
struct Markers<'a> {
    bytes: &'a [u8],
    offset: usize,
    done: bool,
}

impl<'a> Markers<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            offset: 2, // 跳过 SOI
            done: false,
        }
    }
}

impl<'a> Iterator for Markers<'a> {
    type Item = (u8, &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        while !self.done {
            let bytes = self.bytes;
            if self.offset + 1 >= bytes.len() {
                self.done = true;
                return None;
            }
            if bytes[self.offset] != 0xFF {
                // 填充字节，往前挪一格继续找标记。
                self.offset += 1;
                continue;
            }
            let mut marker = bytes[self.offset + 1];
            self.offset += 2;
            while marker == 0xFF && self.offset < bytes.len() {
                marker = bytes[self.offset];
                self.offset += 1;
            }
            match marker {
                // 无负载的标记。
                0xD8 | 0x01 | 0xD0..=0xD7 => continue,
                // 图像结束、扫描开始：都不再有我们关心的头部标记。
                0xD9 | 0xDA => {
                    self.done = true;
                    return None;
                }
                _ => {}
            }
            let Some(length_bytes) = bytes.get(self.offset..self.offset + 2) else {
                self.done = true;
                return None;
            };
            let length = u16::from_be_bytes([length_bytes[0], length_bytes[1]]) as usize;
            if length < 2 || self.offset + length > bytes.len() {
                self.done = true;
                return None;
            }
            let payload = &bytes[self.offset + 2..self.offset + length];
            self.offset += length;
            return Some((marker, payload));
        }
        None
    }
}

fn is_sof(marker: u8) -> bool {
    matches!(
        marker,
        0xC0 | 0xC1 | 0xC2 | 0xC3 | 0xC5 | 0xC6 | 0xC7 | 0xC9 | 0xCA | 0xCB | 0xCD | 0xCE | 0xCF
    )
}

/// 文件准入只需 SOF 尺寸；通用解码器可能先读完整 JPEG，所以在此流式跳过段。
pub(crate) fn dimensions(
    mut reader: impl std::io::Read + std::io::Seek,
) -> std::io::Result<(u32, u32)> {
    use std::io::{Error, ErrorKind, SeekFrom};
    let invalid = || Error::new(ErrorKind::InvalidData, "JPEG 图片头无效或超过 8 MiB");
    reader.seek(SeekFrom::Start(2))?;
    while reader.stream_position()? < 8 * 1024 * 1024 {
        let mut byte = [0];
        reader.read_exact(&mut byte)?;
        if byte[0] != 0xff {
            return Err(invalid());
        }
        loop {
            reader.read_exact(&mut byte)?;
            if byte[0] != 0xff {
                break;
            }
            if reader.stream_position()? >= 8 * 1024 * 1024 {
                return Err(invalid());
            }
        }
        let marker = byte[0];
        if matches!(marker, 0x00 | 0xd8 | 0xd9 | 0xda) {
            return Err(invalid());
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            continue;
        }
        let mut length = [0; 2];
        reader.read_exact(&mut length)?;
        let length = u16::from_be_bytes(length);
        if length < 2 {
            return Err(invalid());
        }
        if is_sof(marker) {
            if length < 8 {
                return Err(invalid());
            }
            let mut payload = [0; 5];
            reader.read_exact(&mut payload)?;
            let width = u16::from_be_bytes([payload[3], payload[4]]) as u32;
            let height = u16::from_be_bytes([payload[1], payload[2]]) as u32;
            if width == 0 || height == 0 {
                return Err(invalid());
            }
            return Ok((width, height));
        }
        reader.seek(SeekFrom::Current(i64::from(length - 2)))?;
    }
    Err(invalid())
}

pub fn inspect(bytes: &[u8]) -> Info {
    let mut info = Info {
        components: 0,
        sampling: None,
        icc: None,
        has_icc: false,
        quality: None,
    };
    let mut tables = Vec::new();
    let mut segments = Vec::new();
    let mut malformed_icc = false;

    for (marker, payload) in Markers::new(bytes) {
        if is_sof(marker) && info.components == 0 {
            // SOF 负载：精度(1) 高(2) 宽(2) 分量数(1)，然后每个分量 id/抽样/量化表 各 1 字节。
            info.components = payload.get(5).copied().unwrap_or(0);
            if info.components == 3 && payload.len() >= 6 + 3 * 3 {
                let mut sampling = [(1u8, 1u8); 3];
                for (index, slot) in sampling.iter_mut().enumerate() {
                    let byte = payload[6 + index * 3 + 1];
                    *slot = (byte >> 4, byte & 0x0F);
                }
                info.sampling = Some(sampling);
            }
        } else if marker == 0xDB {
            tables.push(payload);
        } else if marker == 0xE2 && payload.starts_with(b"ICC_PROFILE\0") {
            info.has_icc = true;
            if payload.len() < 14 {
                malformed_icc = true;
                continue;
            }
            segments.push((payload[12] as usize, payload[13] as usize, &payload[14..]));
        }
    }
    // 先收全序号再判断编号基数，不能用 saturating_sub：0 和 1 会落到同一槽。
    if !malformed_icc && let Some(&(_, count, _)) = segments.first() {
        let zero_based = segments.iter().any(|(sequence, _, _)| *sequence == 0);
        let mut parts = vec![None; count];
        let mut valid = count > 0 && segments.len() == count;
        for (sequence, declared, data) in segments {
            let slot = if zero_based {
                Some(sequence)
            } else {
                sequence.checked_sub(1)
            };
            match slot.filter(|slot| *slot < count) {
                Some(slot) if declared == count && parts[slot].is_none() => {
                    parts[slot] = Some(data)
                }
                _ => valid = false,
            }
        }
        if valid && parts.iter().all(Option::is_some) {
            info.icc = Some(parts.into_iter().flatten().flatten().copied().collect());
        }
    }

    info.quality = estimate_quality(&tables);
    info
}

// 标准量化表只做可信识别，不对自定义表套平均比例猜质量。
fn estimate_quality(payloads: &[&[u8]]) -> Option<u8> {
    const LUMA: [u32; 64] = [
        16, 11, 10, 16, 24, 40, 51, 61, 12, 12, 14, 19, 26, 58, 60, 55, 14, 13, 16, 24, 40, 57, 69,
        56, 14, 17, 22, 29, 51, 87, 80, 62, 18, 22, 37, 56, 68, 109, 103, 77, 24, 35, 55, 64, 81,
        104, 113, 92, 49, 64, 78, 87, 103, 121, 120, 101, 72, 92, 95, 98, 112, 100, 103, 99,
    ];
    const CHROMA: [u32; 64] = [
        17, 18, 24, 47, 99, 99, 99, 99, 18, 21, 26, 66, 99, 99, 99, 99, 24, 26, 56, 99, 99, 99, 99,
        99, 47, 66, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
        99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99, 99,
    ];
    const ZIG: [usize; 64] = [
        0, 1, 8, 16, 9, 2, 3, 10, 17, 24, 32, 25, 18, 11, 4, 5, 12, 19, 26, 33, 40, 48, 41, 34, 27,
        20, 13, 6, 7, 14, 21, 28, 35, 42, 49, 56, 57, 50, 43, 36, 29, 22, 15, 23, 30, 37, 44, 51,
        58, 59, 52, 45, 38, 31, 39, 46, 53, 60, 61, 54, 47, 55, 62, 63,
    ];
    let mut tables = std::collections::BTreeMap::new();
    for payload in payloads {
        let mut at = 0;
        while at < payload.len() {
            let header = payload[at];
            at += 1;
            if header >> 4 != 0 || header & 15 > 1 {
                return None;
            }
            let data = payload.get(at..at + 64)?;
            at += 64;
            if tables.insert(header & 15, data).is_some() {
                return None;
            }
        }
    }
    if !tables.contains_key(&0) {
        return None;
    }
    (1..=100u32)
        .find(|quality| {
            let scale = if *quality < 50 {
                5000 / quality
            } else {
                200 - 2 * quality
            };
            tables.iter().all(|(id, actual)| {
                let base = if *id == 0 { &LUMA } else { &CHROMA };
                actual.iter().enumerate().all(|(i, value)| {
                    *value as u32 == ((base[ZIG[i]] * scale + 50) / 100).clamp(1, 255)
                })
            })
        })
        .map(|quality| quality as u8)
}

pub fn has_icc(bytes: &[u8]) -> bool {
    Markers::new(bytes).any(|(marker, payload)| {
        marker == 0xE2 && payload.starts_with(b"ICC_PROFILE\0") && payload.len() >= 14
    })
}

/// 在 SOI（和可能的 APP0）之后插入 ICC_PROFILE APP2 分段。
///
/// mozjpeg-rs 自带的 `write_icc_profile` 从 0 开始编号，不符合 ICC 的 1 起约定，
/// 别的软件会把它当成坏 profile，所以这里按规范自己分片。
fn inject_icc(bytes: &mut Vec<u8>, profile: &[u8]) {
    if profile.is_empty() || !bytes.starts_with(&[0xFF, 0xD8]) {
        return;
    }
    // 段长字段 16 位，扣掉长度字段 2 字节 + "ICC_PROFILE\0" 12 字节 + 序号/总数 2 字节。
    const MAX_DATA: usize = 65533 - 14;
    let count = profile.len().div_ceil(MAX_DATA);
    // 序号和总数各占 1 字节，超过 255 段就没法表示。
    if count > u8::MAX as usize {
        return;
    }

    let mut insert = Vec::with_capacity(profile.len() + count * 18);
    for (index, chunk) in profile.chunks(MAX_DATA).enumerate() {
        insert.extend_from_slice(&[0xFF, 0xE2]);
        insert.extend_from_slice(&(16 + chunk.len() as u16).to_be_bytes());
        insert.extend_from_slice(b"ICC_PROFILE\0");
        insert.push((index + 1) as u8);
        insert.push(count as u8);
        insert.extend_from_slice(chunk);
    }

    // JFIF 要求 APP0 紧跟 SOI，所以插在 APP0 之后；没有 APP0 就紧跟 SOI。
    let mut at = 2usize;
    if bytes.len() >= 6 && bytes[2] == 0xFF && bytes[3] == 0xE0 {
        let length = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
        if length >= 2 && 4 + length <= bytes.len() {
            at = 4 + length;
        }
    }
    bytes.splice(at..at, insert);
}

fn decode_oriented(input: &[u8]) -> Result<(RgbaImage, bool), String> {
    let reader = ImageReader::new(Cursor::new(input))
        .with_guessed_format()
        .map_err(|error| format!("JPEG 识别失败: {error}"))?;
    let mut decoder = reader
        .into_decoder()
        .map_err(|error| format!("JPEG 解码失败: {error}"))?;
    let orientation = decoder.orientation().unwrap_or(Orientation::NoTransforms);
    let mut image =
        DynamicImage::from_decoder(decoder).map_err(|error| format!("JPEG 解码失败: {error}"))?;
    // 方向烘进像素，输出不再需要 EXIF 的 Orientation。
    image.apply_orientation(orientation);
    let swap_axes = matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    );
    Ok((image.to_rgba8(), swap_axes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::codecs::jpeg::JpegEncoder;
    use image::{ExtendedColorType, Rgb, RgbImage};

    fn gradient() -> RgbImage {
        RgbImage::from_fn(512, 512, |x, y| {
            Rgb([
                (x * 255 / 512) as u8,
                (y * 255 / 512) as u8,
                ((x + y) * 127 / 1024) as u8,
            ])
        })
    }

    fn profile() -> Vec<u8> {
        (0..256u32).map(|index| (index % 251) as u8).collect()
    }

    fn encode_jpeg(image: &RgbImage, quality: f32, icc: Option<&[u8]>) -> Vec<u8> {
        let mut compress = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
        compress.set_size(image.width() as usize, image.height() as usize);
        compress.set_quality(quality);
        if icc.is_some() {
            // 强制 4:4:4，测试里不引入抽样差异。
            compress.set_chroma_sampling_pixel_sizes((1, 1), (1, 1));
        }
        let mut started = compress.start_compress(Vec::new()).unwrap();
        if let Some(profile) = icc {
            started.write_icc_profile(profile);
        }
        started.write_scanlines(image.as_raw()).unwrap();
        started.finish().unwrap()
    }

    #[test]
    fn jpeg_is_compressed() {
        let original = gradient();
        let mut input = Vec::new();
        JpegEncoder::new_with_quality(&mut input, 95)
            .encode(
                original.as_raw(),
                original.width(),
                original.height(),
                ExtendedColorType::Rgb8,
            )
            .unwrap();

        let outcome = squash(&input, &Options::default()).unwrap();
        assert!(outcome.lossy, "应该选择更小的 JPEG 候选");
        assert!(outcome.bytes.len() < input.len());
        assert!(outcome.score.is_finite());
    }

    #[test]
    fn dimension_probe_stops_at_sof_and_rejects_truncated_or_invalid_headers() {
        let image = gradient();
        let input = encode_jpeg(&image, 95.0, None);
        let mut reader = Cursor::new(&input);
        assert_eq!(dimensions(&mut reader).unwrap(), image.dimensions());
        assert!((reader.position() as usize) < input.len());
        for bad in [
            &b"\xff\xd8\xff\xe0\x00\x01"[..],
            &b"\xff\xd8\xff\xd9"[..],
            &input[..6],
        ] {
            assert!(dimensions(Cursor::new(bad)).is_err());
        }
        let marker = input
            .windows(2)
            .position(|pair| pair == [0xff, 0xc0] || pair == [0xff, 0xc2])
            .unwrap();
        let mut zero_height = input.clone();
        zero_height[marker + 5..marker + 7].fill(0);
        assert!(dimensions(Cursor::new(zero_height)).is_err());
    }

    #[test]
    fn icc_is_reencoded_and_reported() {
        let image = gradient();
        let profile = profile();
        let input = encode_jpeg(&image, 95.0, Some(&profile));
        assert_eq!(inspect(&input).icc.as_deref(), Some(&profile[..]));

        let outcome = squash(&input, &Options::default()).unwrap();
        assert!(outcome.lossy);
        assert!(outcome.kept_icc, "重编码后应报告 ICC 已保留");
        assert_eq!(
            inspect(&outcome.bytes).icc.as_deref(),
            Some(&profile[..]),
            "重编码后应把原 ICC 写回 JPEG"
        );
    }

    #[test]
    fn standard_low_quality_is_kept_and_custom_tables_are_not_guessed() {
        let image = gradient();
        for quality in [50, 75, 90] {
            let mut input = Vec::new();
            JpegEncoder::new_with_quality(&mut input, quality)
                .encode(
                    image.as_raw(),
                    image.width(),
                    image.height(),
                    ExtendedColorType::Rgb8,
                )
                .unwrap();
            assert_eq!(inspect(&input).quality, Some(quality));
            if quality <= 75 {
                let output = squash(&input, &Options::default()).unwrap();
                assert_eq!(output.bytes, input);
                assert!(!output.lossy);
            }
            // 修改 DQT 的一个系数，不能再认定它属于标准质量档。
            let table = input.windows(2).position(|v| v == [0xff, 0xdb]).unwrap();
            input[table + 5] = 237;
            assert_eq!(inspect(&input).quality, None);
        }
        let custom = encode_jpeg(&image, 95.0, None);
        assert_eq!(inspect(&custom).quality, None, "mozjpeg 自定义表应放行");
    }

    #[test]
    fn zero_based_sequence_is_accepted() {
        // 旧版 app 用 mozjpeg 自带的 writer，序号从 0 开始；这种老输出也要能读回。
        let profile = vec![7u8; 70_000];
        let mut input = vec![0xFF, 0xD8];
        inject_icc(&mut input, &profile);
        // 覆盖多分段；只有一段时 0/1 都落到槽 0，会掩盖旧解析器的错误。
        let positions: Vec<_> = input
            .windows(12)
            .enumerate()
            .filter_map(|(i, w)| (w == b"ICC_PROFILE\0").then_some(i + 12))
            .collect();
        for position in positions {
            input[position] -= 1;
        }
        assert!(inspect(&input).icc.as_deref() == Some(profile.as_slice()));
    }

    #[test]
    fn icc_round_trips_across_chunks() {
        let mut bytes = vec![0xFF, 0xD8];
        let profile: Vec<u8> = (0..70_000u32).map(|index| (index % 253) as u8).collect();
        inject_icc(&mut bytes, &profile);
        assert!(has_icc(&bytes));
        assert_eq!(inspect(&bytes).icc.as_deref(), Some(&profile[..]));
    }

    #[test]
    fn duplicate_icc_segments_are_not_silently_overwritten() {
        let mut bytes = encode_jpeg(&gradient(), 95.0, None);
        inject_icc(&mut bytes, &vec![7; 70_000]);
        let positions: Vec<_> = bytes
            .windows(12)
            .enumerate()
            .filter_map(|(i, w)| (w == b"ICC_PROFILE\0").then_some(i + 12))
            .collect();
        bytes[positions[1]] = bytes[positions[0]];
        let info = inspect(&bytes);
        assert!(info.has_icc && info.icc.is_none());
        assert_eq!(squash(&bytes, &Options::default()).unwrap().bytes, bytes);
    }

    #[test]
    fn chroma_subsampling_is_preserved() {
        // 4:4:4 的输入不能被降到 4:2:0，否则截图里的彩色文字会糊。
        let image = gradient();
        let input = encode_jpeg(&image, 95.0, Some(&profile()));
        let sampling = inspect(&input).sampling.expect("3 分量应有抽样信息");
        assert_eq!(sampling, [(1, 1); 3], "fixture 应是 4:4:4");

        let outcome = squash(&input, &Options::default()).unwrap();
        assert_eq!(
            inspect(&outcome.bytes).sampling,
            Some([(1, 1); 3]),
            "重编码后应保持 4:4:4"
        );
    }

    #[test]
    fn chroma_pixel_size_maps_the_common_layouts() {
        assert_eq!(chroma_pixel_size((1, 1), (1, 1)), (1, 1), "4:4:4");
        assert_eq!(chroma_pixel_size((2, 1), (1, 1)), (2, 1), "4:2:2");
        assert_eq!(chroma_pixel_size((2, 2), (1, 1)), (2, 2), "4:2:0");
    }

    #[test]
    fn lossless_mode_leaves_jpeg_alone() {
        let input = encode_jpeg(&gradient(), 95.0, None);
        let options = Options {
            mode: Mode::Lossless,
            output_format: squash::OutputFormat::WebP,
            ..Options::default()
        };
        let outcome = squash(&input, &options).unwrap();
        assert_eq!(outcome.method, "original");
        assert_eq!(outcome.bytes, input);
        assert!(outcome.hint.unwrap().contains("无损"));
    }

    #[test]
    fn markers_stop_before_the_scan() {
        let input = encode_jpeg(&gradient(), 80.0, None);
        // 走到 SOS 就停，不会把熵数据当成标记解析。
        let count = Markers::new(&input).count();
        assert!(count >= 2, "至少应该有 SOF 和 DQT，实际 {count}");
        assert!(inspect(&input).components == 3);
    }
}
