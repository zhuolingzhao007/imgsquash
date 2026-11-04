//! 大图路径的边界与语义回归；固定预算可在小样本上触发实际流式调用链。
use crate::{
    file_plan::{Header, Route},
    pipeline::Payload,
    resources::Budget,
    squash::{self, Mode, Options, OutputFormat},
    stream_jpeg, stream_png,
};
use image::{ImageFormat, Rgba, RgbaImage};
use std::{
    fs::File,
    io::{Cursor, Read, Seek, Write},
};

fn png_bytes(image: &RgbaImage) -> Vec<u8> {
    let mut bytes = Vec::new();
    image
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .unwrap();
    bytes
}

#[test]
fn streamed_transparency_and_lossless_mode_remain_pixel_exact() {
    let dir = tempfile::tempdir().unwrap();
    let image = RgbaImage::from_fn(350, 233, |x, y| {
        Rgba([x as u8, y as u8, 123, ((x + y) % 256) as u8])
    });
    let input = dir.path().join("alpha.png");
    std::fs::write(&input, png_bytes(&image)).unwrap();
    let mut source = File::open(&input).unwrap();
    let header = Header::read(&mut source).unwrap();
    let result = crate::streaming::run(
        &mut source,
        &header,
        header.source_len,
        &dir.path().join("out.png"),
        &Options {
            mode: Mode::Lossless,
            ..Options::default()
        },
        false,
    )
    .unwrap();
    assert!(!result.lossy);
    assert_eq!(image::open(result.output_path).unwrap().to_rgba8(), image);
}

#[test]
fn large_plan_selects_original_lossless_or_streaming_before_reserving_memory() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("icon.png");
    std::fs::write(&input, include_bytes!("../tests/fixtures/icon.png")).unwrap();
    let header = Header::read(&mut File::open(&input).unwrap()).unwrap();
    assert!(matches!(
        header.route(header.source_len, &Options::default(), 1024 * 1024),
        Route::Original(_)
    ));
    assert!(matches!(
        header.route(header.source_len, &Options::default(), 64 * 1024 * 1024),
        Route::Memory { .. }
    ));
    assert!(matches!(
        header.route(200 * 1024 * 1024, &Options::default(), 256 * 1024 * 1024),
        Route::Stream { lossy: true, .. }
    ));
    assert!(matches!(
        header.route(200 * 1024 * 1024, &Options::default(), 64 * 1024 * 1024),
        Route::Stream { lossy: false, .. }
    ));
    std::fs::OpenOptions::new()
        .append(true)
        .open(&input)
        .unwrap()
        .write_all(b"x")
        .unwrap();
    assert!(header.unchanged(&File::open(input).unwrap()).is_err());
}

#[test]
fn a_small_budget_preserves_original_without_decoding_or_overwriting() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("icon.png");
    let bytes = include_bytes!("../tests/fixtures/icon-icc.png");
    std::fs::write(&input, bytes).unwrap();
    let output = dir.path().join("out.png");
    std::fs::write(&output, b"existing").unwrap();
    let result = squash::compress_file_with_budget(
        &input,
        &output,
        &Options::default(),
        &Budget::new(1024 * 1024),
    )
    .unwrap();
    assert_eq!(result.method, "original");
    assert!(result.kept_icc);
    assert_eq!(std::fs::read(result.output_path).unwrap(), bytes);
    assert_eq!(std::fs::read(output).unwrap(), b"existing");
}

/// 独立生成 Adam7 测试图；zlib stored blocks 避免再引入测试编码依赖。
fn adam7(width: u32, height: u32, depth: u8, color: u8) -> Vec<u8> {
    let samples = if color == 6 { 4 } else { 1 };
    let bits = samples * depth as usize;
    let mut raw = Vec::new();
    for (x0, y0, dx, dy) in [
        (0, 0, 8, 8),
        (4, 0, 8, 8),
        (0, 4, 4, 8),
        (2, 0, 4, 4),
        (0, 2, 2, 4),
        (1, 0, 2, 2),
        (0, 1, 1, 2),
    ] {
        if x0 >= width || y0 >= height {
            continue;
        }
        for y in (y0..height).step_by(dy as usize) {
            raw.push(0);
            let mut row = vec![0u8; (((width - x0).div_ceil(dx)) as usize * bits).div_ceil(8)];
            for (i, x) in (x0..width).step_by(dx as usize).enumerate() {
                match (color, depth) {
                    (6, 8) => row[i * 4..i * 4 + 4].copy_from_slice(&[
                        x as u8 * 11,
                        y as u8 * 13,
                        90,
                        (x + y) as u8 * 7,
                    ]),
                    (0, 16) => row[i * 2..i * 2 + 2]
                        .copy_from_slice(&((x * 500 + y * 31) as u16).to_be_bytes()),
                    (3, 1) => row[i / 8] |= ((x + y) as u8 % 2) << (7 - i % 8),
                    _ => unreachable!(),
                }
            }
            raw.extend(row);
        }
    }
    let mut zlib = vec![0x78, 0x01];
    let chunks = raw.chunks(65535);
    let count = chunks.len();
    for (i, chunk) in chunks.enumerate() {
        zlib.push(u8::from(i + 1 == count));
        zlib.extend((chunk.len() as u16).to_le_bytes());
        zlib.extend((!(chunk.len() as u16)).to_le_bytes());
        zlib.extend(chunk);
    }
    let (mut a, mut b) = (1u32, 0u32);
    for v in &raw {
        a = (a + u32::from(*v)) % 65521;
        b = (b + a) % 65521;
    }
    zlib.extend(((b << 16) | a).to_be_bytes());
    let mut result = tinypng_core::png_meta::SIGNATURE.to_vec();
    let mut ihdr = Vec::new();
    ihdr.extend(width.to_be_bytes());
    ihdr.extend(height.to_be_bytes());
    ihdr.extend([depth, color, 0, 0, 1]);
    tinypng_core::png_meta::write_chunk(&mut result, b"IHDR", &ihdr);
    if color == 3 {
        tinypng_core::png_meta::write_chunk(&mut result, b"PLTE", &[30, 60, 90, 200, 160, 120]);
        tinypng_core::png_meta::write_chunk(&mut result, b"tRNS", &[0, 180]);
        tinypng_core::png_meta::write_chunk(&mut result, b"bKGD", &[1]);
    }
    tinypng_core::png_meta::write_chunk(&mut result, b"IDAT", &zlib);
    tinypng_core::png_meta::write_chunk(&mut result, b"eXIf", b"test-exif");
    tinypng_core::png_meta::write_chunk(&mut result, b"IEND", &[]);
    result
}

#[test]
fn adam7_streaming_preserves_rgba_16bit_palette_and_post_idat_metadata() {
    let dir = tempfile::tempdir().unwrap();
    for (depth, color) in [(8, 6), (16, 0), (1, 3)] {
        let bytes = adam7(13, 11, depth, color);
        let expected = image::load_from_memory(&bytes).unwrap().to_rgba16();
        let path = dir.path().join(format!("source-{depth}-{color}.png"));
        std::fs::write(&path, &bytes).unwrap();
        let mut source = File::open(&path).unwrap();
        let header = Header::read(&mut source).unwrap();
        let mut pixels = stream_png::decode(&mut source, false).unwrap();
        let mut artifact = stream_png::encode(
            &mut pixels,
            header.png.as_ref().unwrap(),
            &dir.path().join("output.png"),
            false,
        )
        .unwrap();
        assert!(artifact.byte_len() > 0);
        artifact.file.rewind().unwrap();
        let mut output = Vec::new();
        artifact.file.read_to_end(&mut output).unwrap();
        assert_eq!(
            image::load_from_memory(&output).unwrap().to_rgba16(),
            expected
        );
        assert_eq!(
            Header::read(artifact.file.as_file_mut())
                .unwrap()
                .png
                .unwrap()
                .metadata
                .iter()
                .find(|(kind, _)| kind == b"eXIf")
                .unwrap()
                .1,
            b"test-exif"
        );
        if depth == 8 {
            let mut rgba = stream_png::decode(&mut source, true).unwrap();
            let all = rgba
                .region(tinypng_core::quality::Region {
                    x: 0,
                    y: 0,
                    width: 13,
                    height: 11,
                })
                .unwrap();
            assert_eq!(all, image::load_from_memory(&bytes).unwrap().to_rgba8());
        }
    }
}

#[test]
fn png_icc_survives_streamed_lossless_encoding() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("icc.png");
    std::fs::write(&path, include_bytes!("../tests/fixtures/icon-icc.png")).unwrap();
    let mut source = File::open(&path).unwrap();
    let header = Header::read(&mut source).unwrap();
    let mut pixels = stream_png::decode(&mut source, false).unwrap();
    let mut output = stream_png::encode(
        &mut pixels,
        header.png.as_ref().unwrap(),
        &dir.path().join("out.png"),
        false,
    )
    .unwrap();
    let result = Header::read(output.file.as_file_mut()).unwrap();
    assert!(result.has_icc());
    assert_eq!(
        header
            .png
            .unwrap()
            .metadata
            .iter()
            .find(|(k, _)| k == b"iCCP"),
        result
            .png
            .unwrap()
            .metadata
            .iter()
            .find(|(k, _)| k == b"iCCP")
    );
}

#[test]
fn jpeg_scanlines_preserve_size_and_exif_orientation() {
    let dir = tempfile::tempdir().unwrap();
    let image = image::load_from_memory(include_bytes!("../tests/fixtures/photo.png")).unwrap();
    let mut bytes = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 95)
        .encode_image(&image)
        .unwrap();
    let exif = b"Exif\0\0II\x2a\0\x08\0\0\0\x01\0\x12\x01\x03\0\x01\0\0\0\x06\0\0\0\0\0\0\0";
    let mut marker = vec![0xff, 0xe1];
    marker.extend(((exif.len() + 2) as u16).to_be_bytes());
    marker.extend(exif);
    bytes.splice(2..2, marker);
    let path = dir.path().join("photo.jpg");
    std::fs::write(&path, &bytes).unwrap();
    let mut source = File::open(path).unwrap();
    let mut header = Header::read(&mut source).unwrap();
    header.jpeg.as_mut().unwrap().density = Some(mozjpeg::PixelDensity {
        unit: mozjpeg::PixelDensityUnit::Inches,
        x: 300,
        y: 150,
    });
    let mut pixels = stream_jpeg::decode(&mut source).unwrap();
    let sample = pixels
        .region(tinypng_core::quality::Region {
            x: 0,
            y: 0,
            width: pixels.width,
            height: pixels.height,
        })
        .unwrap();
    let mut output = stream_jpeg::encode(
        &mut pixels,
        header.jpeg.as_ref().unwrap(),
        90,
        &dir.path().join("out.jpg"),
    )
    .unwrap();
    let actual = Header::read(output.file.as_file_mut()).unwrap();
    assert_eq!((actual.width, actual.height), (header.width, header.height));
    let actual = actual.jpeg.unwrap();
    let original = header.jpeg.unwrap();
    assert_eq!(actual.markers, original.markers);
    let density = actual.density.unwrap();
    assert_eq!((density.x, density.y), (300, 150));
    assert!(matches!(density.unit, mozjpeg::PixelDensityUnit::Inches));
    assert_eq!(
        std::fs::read(output.file.path()).unwrap(),
        stream_jpeg::encode_sample(&sample, &original, 90).unwrap(),
        "样本与逐行编码必须使用相同的码流配置"
    );
    let broken = dir.path().join("broken.jpg");
    let mut data = bytes[..bytes.len() / 2].to_vec();
    data.extend([0xff, 0xd9]);
    std::fs::write(&broken, data).unwrap();
    assert!(stream_jpeg::decode(&mut File::open(broken).unwrap()).is_err());
    // 只替换 SOF 来测试规划器；算术渐进也必须预留整图系数，不在此解码测试数据。
    let mut arithmetic = bytes;
    let sof = arithmetic
        .windows(2)
        .position(|p| p == [0xff, 0xc0] || p == [0xff, 0xc2])
        .unwrap();
    arithmetic[sof + 1] = 0xca;
    let path = dir.path().join("arithmetic-header.jpg");
    std::fs::write(&path, arithmetic).unwrap();
    let mut header = Header::read(&mut File::open(path).unwrap()).unwrap();
    assert!(header.jpeg.as_ref().unwrap().progressive);
    header.width = 60_000;
    header.height = 60_000;
    assert!(matches!(
        header.route(header.source_len, &Options::default(), 512 * 1024 * 1024),
        Route::Original(_)
    ));
}

#[test]
#[ignore = "release 真实纹理大 JPEG 验证"]
fn bench_24mp_jpeg_streaming() {
    let temporary = tempfile::tempdir().unwrap();
    let dir = std::env::var_os("TINYPNG_LARGE_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| temporary.path().to_owned());
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("24mp-photo.jpg");
    {
        let photo = image::load_from_memory(include_bytes!("../tests/fixtures/photo.png"))
            .unwrap()
            .to_rgb8();
        let mut encoder = mozjpeg::Compress::new(mozjpeg::ColorSpace::JCS_RGB);
        encoder.set_fastest_defaults();
        encoder.set_size(6000, 4000);
        encoder.set_quality(95.0);
        encoder.set_optimize_scans(false);
        encoder.set_optimize_coding(false);
        let mut writer = encoder
            .start_compress(File::create(&input).unwrap())
            .unwrap();
        let mut row = vec![0u8; 6000 * 3];
        for y in 0..4000 {
            for (x, pixel) in row.as_chunks_mut::<3>().0.iter_mut().enumerate() {
                pixel.copy_from_slice(
                    &photo
                        .get_pixel(x as u32 % photo.width(), y % photo.height())
                        .0,
                );
            }
            writer.write_scanlines(&row).unwrap();
        }
        writer.finish().unwrap();
    }
    let start = std::time::Instant::now();
    let preset = tinypng_core::policy::Preset::parse(
        &std::env::var("TINYPNG_LARGE_PRESET").unwrap_or_else(|_| "standard".into()),
    )
    .unwrap();
    let result = squash::compress_file(
        &input,
        &dir.join("24mp-photo-output.jpg"),
        &Options {
            policy: preset.policy(),
            ..Options::default()
        },
    )
    .unwrap();
    assert!(result.output_size <= result.input_size);
    assert_eq!(
        squash::image_dimensions(std::io::BufReader::new(
            File::open(&result.output_path).unwrap()
        ))
        .unwrap(),
        (6000, 4000)
    );
    println!(
        "24MP JPEG: {} -> {} B, method={}, score={} kind={} lossy={} {:.2}s path={}",
        result.input_size,
        result.output_size,
        result.method,
        result.score,
        result.score_kind,
        result.lossy,
        start.elapsed().as_secs_f64(),
        result.output_path
    );
}

#[test]
fn a_restricted_budget_uses_streamed_png_and_preserves_originals() {
    let dir = tempfile::tempdir().unwrap();
    let input = dir.path().join("ui.png");
    let original = include_bytes!("../design/layout-1180.png");
    std::fs::write(&input, original).unwrap();
    let options = Options {
        output_format: OutputFormat::WebP,
        ..Options::default()
    };
    let result = squash::compress_file_with_budget(
        &input,
        &dir.path().join("out.webp"),
        &options,
        &Budget::new(220 * 1024 * 1024),
    )
    .unwrap();
    assert!(result.output_path.ends_with(".png"));
    assert!(result.output_size <= original.len() as u64);
    assert!(result.hint.unwrap().contains("低内存"));
    assert_eq!(std::fs::read(input).unwrap(), original);
    assert_eq!(image::open(result.output_path).unwrap().width(), 2360);
}

#[test]
#[ignore = "release 大图性能验证：TINYPNG_LARGE_OUT 指定可保留的输出目录"]
fn bench_48mp_streaming() {
    let temporary = tempfile::tempdir().unwrap();
    let dir = std::env::var_os("TINYPNG_LARGE_OUT")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| temporary.path().to_owned());
    std::fs::create_dir_all(&dir).unwrap();
    let input = dir.join("48mp.png");
    {
        let file = File::create(&input).unwrap();
        let mut encoder = png::Encoder::new(file, 8000, 6000);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_compression(png::Compression::Fast);
        let mut writer = encoder
            .write_header()
            .unwrap()
            .into_stream_writer()
            .unwrap();
        let row: Vec<u8> = (0..8000)
            .flat_map(|x| {
                if x / 128 % 2 == 0 {
                    [20, 80, 140, 255]
                } else {
                    [200, 160, 120, 255]
                }
            })
            .collect();
        for _ in 0..6000 {
            writer.write_all(&row).unwrap();
        }
        writer.finish().unwrap();
    }
    let started = std::time::Instant::now();
    let result =
        squash::compress_file(&input, &dir.join("48mp-output.png"), &Options::default()).unwrap();
    assert!(!result.lossy);
    assert!(result.output_size < result.input_size);
    assert_eq!(
        squash::image_dimensions(std::io::BufReader::new(
            File::open(&result.output_path).unwrap()
        ))
        .unwrap(),
        (8000, 6000)
    );
    let processing_time = started.elapsed();
    let mut a = stream_png::decode(&mut File::open(&input).unwrap(), true).unwrap();
    let mut b = stream_png::decode(&mut File::open(&result.output_path).unwrap(), true).unwrap();
    let mut row_a = vec![0; a.stride()];
    let mut row_b = vec![0; b.stride()];
    for y in 0..a.height {
        a.row(y, &mut row_a).unwrap();
        b.row(y, &mut row_b).unwrap();
        assert_eq!(row_a, row_b, "保存后的第 {y} 行像素不相同");
    }
    println!(
        "48MP: {} -> {} B, method={}, {:.2}s, path={}",
        result.input_size,
        result.output_size,
        result.method,
        processing_time.as_secs_f64(),
        result.output_path
    );
}
