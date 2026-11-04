//! PNG 扫描线解码/编码；Adam7 在磁盘逐行合并，不分配完整帧。
use crate::{file_plan::Png, raster::Raster, streaming::Artifact};
use std::{
    fs::File,
    io::{BufReader, Read, Seek, Write},
    path::Path,
};

fn decode_error(error: png::DecodingError) -> String {
    if matches!(error, png::DecodingError::LimitsExceeded) {
        format!("低内存资源限制：{error}")
    } else {
        format!("PNG 解码失败: {error}")
    }
}

pub(crate) fn decode(source: &mut File, rgba: bool) -> Result<Raster, String> {
    source.rewind().map_err(|e| e.to_string())?;
    let (width, _) = crate::squash::image_dimensions(BufReader::new(&mut *source))?;
    source.rewind().map_err(|e| e.to_string())?;
    let mut decoder = png::Decoder::new_with_limits(
        BufReader::new(source),
        png::Limits {
            bytes: 32 * 1024 * 1024 + width as usize * 16,
        },
    );
    if rgba {
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    }
    let mut reader = decoder.read_info().map_err(decode_error)?;
    let (color, depth) = reader.output_color_type();
    let (w, h) = (reader.info().width, reader.info().height);
    let interlaced = reader.info().interlaced;
    let mut raster = Raster::new(
        w,
        h,
        if rgba { png::ColorType::Rgba } else { color },
        if rgba { png::BitDepth::Eight } else { depth },
    )?;
    if !rgba {
        raster.palette = reader.info().palette.as_ref().map(|v| v.to_vec());
        raster.trns = reader.info().trns.as_ref().map(|v| v.to_vec());
    }
    let passes: &[(u32, u32, u32, u32)] = if interlaced {
        &[
            (0, 0, 8, 8),
            (4, 0, 8, 8),
            (0, 4, 4, 8),
            (2, 0, 4, 4),
            (0, 2, 2, 4),
            (1, 0, 2, 2),
            (0, 1, 1, 2),
        ]
    } else {
        &[(0, 0, 1, 1)]
    };
    let bpp = if rgba {
        32
    } else {
        color.samples() * depth as usize
    };
    let mut full = vec![0; raster.stride()];
    for &(x0, y0, dx, dy) in passes {
        if x0 >= w || y0 >= h {
            continue;
        }
        for y in (y0..h).step_by(dy as usize) {
            let row = reader
                .next_interlaced_row()
                .map_err(decode_error)?
                .ok_or("PNG 像素行不完整")?;
            let data = if rgba {
                to_rgba(row.data(), color)?
            } else {
                row.data().to_vec()
            };
            if dx == 1 && x0 == 0 {
                raster.write_row(y, &data)?;
                continue;
            }
            raster.row(y, &mut full)?;
            let samples = (w - x0).div_ceil(dx) as usize;
            for i in 0..samples {
                let target = (x0 as usize + i * dx as usize) * bpp;
                if bpp >= 8 {
                    let n = bpp / 8;
                    full[target / 8..target / 8 + n].copy_from_slice(&data[i * n..i * n + n]);
                } else {
                    let src = i * bpp;
                    let value = (data[src / 8] >> (8 - bpp - src % 8)) & ((1 << bpp) - 1);
                    let shift = 8 - bpp - target % 8;
                    full[target / 8] =
                        (full[target / 8] & !(((1 << bpp) - 1) << shift)) | (value << shift);
                }
            }
            raster.write_row(y, &full)?;
        }
    }
    if reader
        .next_interlaced_row()
        .map_err(decode_error)?
        .is_some()
    {
        return Err("PNG 行数超出尺寸".into());
    }
    reader.finish().map_err(decode_error)?;
    Ok(raster)
}
fn to_rgba(data: &[u8], color: png::ColorType) -> Result<Vec<u8>, String> {
    let mut rgba = Vec::with_capacity(data.len() / color.samples() * 4);
    for p in data.chunks_exact(color.samples()) {
        rgba.extend_from_slice(&match color {
            png::ColorType::Rgba => [p[0], p[1], p[2], p[3]],
            png::ColorType::Rgb => [p[0], p[1], p[2], 255],
            png::ColorType::Grayscale => [p[0], p[0], p[0], 255],
            png::ColorType::GrayscaleAlpha => [p[0], p[0], p[0], p[1]],
            _ => return Err("索引 PNG 未展开".into()),
        });
    }
    Ok(rgba)
}

pub(crate) fn encode(
    raster: &mut Raster,
    header: &Png,
    target: &Path,
    quantized: bool,
) -> Result<Artifact, String> {
    let artifact = Artifact::new(target)?;
    {
        let mut encoder = png::Encoder::new(
            artifact.file.reopen().map_err(|e| e.to_string())?,
            raster.width,
            raster.height,
        );
        encoder.set_color(raster.color);
        encoder.set_depth(raster.depth);
        if let Some(palette) = &raster.palette {
            encoder.set_palette(palette.as_slice());
        }
        if let Some(trns) = &raster.trns {
            encoder.set_trns(trns.as_slice());
        }
        encoder.set_compression(if quantized {
            png::Compression::Balanced
        } else {
            png::Compression::Fast
        });
        let mut png_writer = encoder.write_header().map_err(|e| e.to_string())?;
        {
            let mut writer = png_writer.stream_writer().map_err(|e| e.to_string())?;
            let mut row = vec![0; raster.stride()];
            for y in 0..raster.height {
                raster.row(y, &mut row)?;
                writer.write_all(&row).map_err(|e| e.to_string())?;
            }
            writer.finish().map_err(|e| e.to_string())?;
        }
        png_writer.finish().map_err(|e| e.to_string())?;
    }
    metadata(artifact, header, target, quantized)
}

/// 将辅助块流式回贴到 PLTE/IDAT 前，保留原始配置字节，避免重新读入整个编码文件。
pub(crate) fn metadata(
    mut encoded: Artifact,
    header: &Png,
    target: &Path,
    quantized: bool,
) -> Result<Artifact, String> {
    if !header.metadata.iter().any(|(kind, _)| {
        kind != b"tRNS" && (!quantized || tinypng_core::png_meta::PRESERVED.contains(&kind))
    }) {
        encoded.refresh()?;
        return Ok(encoded);
    }
    let mut output = Artifact::new(target)?;
    encoded.file.rewind().map_err(|e| e.to_string())?;
    let mut signature = [0; 8];
    encoded
        .file
        .read_exact(&mut signature)
        .map_err(|e| e.to_string())?;
    output
        .file
        .write_all(&signature)
        .map_err(|e| e.to_string())?;
    let mut before = false;
    let mut after = false;
    loop {
        let mut chunk = [0; 8];
        encoded
            .file
            .read_exact(&mut chunk)
            .map_err(|e| e.to_string())?;
        let size = u32::from_be_bytes(chunk[..4].try_into().unwrap()) as u64;
        let kind = &chunk[4..];
        if !before && (kind == b"PLTE" || kind == b"IDAT") {
            write_metadata(&mut output.file, header, quantized, false)?;
            before = true;
        }
        if !after && kind == b"IDAT" {
            write_metadata(&mut output.file, header, quantized, true)?;
            after = true;
        }
        output.file.write_all(&chunk).map_err(|e| e.to_string())?;
        if std::io::copy(&mut (&mut encoded.file).take(size + 4), &mut output.file)
            .map_err(|e| e.to_string())?
            != size + 4
        {
            return Err("PNG 编码块不完整".into());
        }
        if kind == b"IEND" {
            break;
        }
    }
    output.refresh()?;
    Ok(output)
}
fn write_metadata(
    output: &mut impl Write,
    header: &Png,
    quantized: bool,
    after_palette: bool,
) -> Result<(), String> {
    for (kind, data) in &header.metadata {
        if kind == b"tRNS"
            || [b"bKGD", b"hIST"].contains(&kind) != after_palette
            || quantized && !tinypng_core::png_meta::PRESERVED.contains(&kind)
        {
            continue;
        }
        let mut bytes = Vec::new();
        tinypng_core::png_meta::write_chunk(&mut bytes, kind, data);
        output.write_all(&bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}
