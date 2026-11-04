//! JPEG 扫描线读写；编码不启用需要整图系数搜索的渐进/优化编码。
use crate::{file_plan::Jpeg, raster::Raster, streaming::Artifact};
use std::{
    fs::File,
    io::{BufReader, Seek},
    path::Path,
};

pub(crate) fn decode(source: &mut File) -> Result<Raster, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| decode_checked(source)))
        .map_err(|_| "JPEG 数据损坏，未采用重编码结果".to_string())?
}

extern "C-unwind" fn fatal_jpeg(_info: &mut mozjpeg_sys::jpeg_common_struct) {
    std::panic::resume_unwind(Box::new("JPEG 数据损坏"));
}
extern "C-unwind" fn jpeg_message(
    info: &mut mozjpeg_sys::jpeg_common_struct,
    level: std::ffi::c_int,
) {
    if level < 0 {
        fatal_jpeg(info);
    }
}

fn decode_checked(source: &mut File) -> Result<Raster, String> {
    source.rewind().map_err(|e| e.to_string())?;
    // SAFETY: jpeg_error_mgr 是整数/数组/可空回调的 C POD；标准初始化后替换可展开的回调。
    let mut errors: mozjpeg_sys::jpeg_error_mgr = unsafe { std::mem::zeroed() };
    unsafe {
        mozjpeg_sys::jpeg_std_error(&mut errors);
    }
    errors.error_exit = Some(fatal_jpeg);
    errors.emit_message = Some(jpeg_message);
    let mut decoder = mozjpeg::Decompress::with_err(errors)
        .from_reader(BufReader::new(source))
        .map_err(|e| format!("JPEG 解码失败: {e}"))?
        .rgba()
        .map_err(|e| e.to_string())?;
    let mut raster = Raster::new(
        decoder.width() as u32,
        decoder.height() as u32,
        png::ColorType::Rgba,
        png::BitDepth::Eight,
    )?;
    let mut row = vec![0u8; raster.stride()];
    for y in 0..raster.height {
        decoder
            .read_scanlines_into(&mut row)
            .map_err(|e| format!("JPEG 像素读取失败: {e}"))?;
        raster.write_row(y, &row)?;
    }
    decoder.finish().map_err(|e| e.to_string())?;
    Ok(raster)
}
pub(crate) fn encode(
    raster: &mut Raster,
    header: &Jpeg,
    quality: u8,
    target: &Path,
) -> Result<Artifact, String> {
    let mut output = Artifact::new(target)?;
    let encoder = configured(raster.width, raster.height, header, quality);
    let mut encoder = encoder
        .start_compress(output.file.as_file_mut())
        .map_err(|e| e.to_string())?;
    for (kind, data) in &header.markers {
        encoder.write_marker(mozjpeg::Marker::from(*kind), data);
    }
    let mut rgba = vec![0; raster.stride()];
    let mut rgb = Vec::with_capacity(raster.width as usize * 3);
    for y in 0..raster.height {
        raster.row(y, &mut rgba)?;
        rgb.clear();
        rgb.extend(
            rgba.as_chunks::<4>()
                .0
                .iter()
                .flat_map(|p| [p[0], p[1], p[2]]),
        );
        encoder.write_scanlines(&rgb).map_err(|e| e.to_string())?;
    }
    encoder.finish().map_err(|e| e.to_string())?;
    output.refresh()?;
    Ok(output)
}

/// 搜索使用与最终扫描线编码相同的熵编码设置，避免高估渐进/优化编码的收益。
pub(crate) fn encode_sample(
    sample: &image::RgbaImage,
    header: &Jpeg,
    quality: u8,
) -> Result<Vec<u8>, String> {
    let encoder = configured(sample.width(), sample.height(), header, quality);
    let mut encoder = encoder
        .start_compress(Vec::new())
        .map_err(|e| e.to_string())?;
    for (kind, data) in &header.markers {
        encoder.write_marker(mozjpeg::Marker::from(*kind), data);
    }
    let rgb: Vec<u8> = sample.pixels().flat_map(|p| [p[0], p[1], p[2]]).collect();
    encoder.write_scanlines(&rgb).map_err(|e| e.to_string())?;
    encoder.finish().map_err(|e| e.to_string())
}

fn configured(width: u32, height: u32, header: &Jpeg, quality: u8) -> mozjpeg::Compress {
    let mut encoder = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
    // MozJPEG 默认 trellis 也会申请整图系数缓冲；仅关闭渐进和 Huffman 优化还不够。
    encoder.set_fastest_defaults();
    encoder.set_size(width as usize, height as usize);
    encoder.set_quality(quality as f32);
    encoder.set_optimize_scans(false);
    encoder.set_optimize_coding(false);
    if let Some(d) = &header.density {
        encoder.set_pixel_density(mozjpeg::PixelDensity {
            unit: d.unit,
            x: d.x,
            y: d.y,
        });
    }
    if let Some(s) = header.info.sampling {
        encoder.set_chroma_sampling_pixel_sizes(
            crate::jpeg::chroma_pixel_size(s[0], s[1]),
            crate::jpeg::chroma_pixel_size(s[0], s[2]),
        );
    }
    encoder
}
