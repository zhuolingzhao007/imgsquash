//! 手动执行的实图/边界验收，输出可复查的原图、结果、CSV 和并排预览。
use crate::squash::{self, Mode, Options, OutputFormat};
use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
use std::{borrow::Cow, io::Cursor, path::PathBuf, time::Instant};
use tinypng_core::{policy::Preset, quality};

fn png(image: &RgbaImage, profile: Option<&[u8]>) -> Vec<u8> {
    let mut info = ::png::Info::with_size(image.width(), image.height());
    info.color_type = ::png::ColorType::Rgba;
    info.bit_depth = ::png::BitDepth::Eight;
    info.icc_profile = profile.map(Cow::Borrowed);
    let mut bytes = Vec::new();
    let encoder = ::png::Encoder::with_info(&mut bytes, info).unwrap();
    encoder
        .write_header()
        .unwrap()
        .write_image_data(image.as_raw())
        .unwrap();
    bytes
}

#[test]
#[ignore = "release 实图验收及可视化产物：scripts/quality-check.sh"]
fn acceptance_corpus() {
    let directory = PathBuf::from(
        std::env::var_os("TINYPNG_ACCEPTANCE_OUT").unwrap_or_else(|| ".nsh/out/quality".into()),
    );
    std::fs::create_dir_all(&directory).unwrap();
    let gradient = RgbaImage::from_fn(768, 256, |x, y| {
        Rgba([
            (25 + x * 180 / 767) as u8,
            (40 + y * 170 / 255) as u8,
            (80 + (x + y) * 100 / 1022) as u8,
            255,
        ])
    });
    let alpha = RgbaImage::from_fn(512, 256, |x, y| {
        let distance = ((x as f64 - 256.0).powi(2) + (y as f64 - 128.0).powi(2)).sqrt();
        Rgba([
            (x / 2) as u8,
            y as u8,
            170,
            ((120.0 - distance) * 16.0).clamp(0.0, 255.0) as u8,
        ])
    });
    let texture = RgbaImage::from_fn(512, 320, |x, y| {
        let noise = x
            .wrapping_mul(1_103_515_245)
            .wrapping_add(y.wrapping_mul(12_345));
        Rgba([
            (noise >> 8) as u8,
            (noise >> 16) as u8,
            (noise >> 24) as u8,
            255,
        ])
    });
    let photo = include_bytes!("../tests/fixtures/photo.png").to_vec();
    let photo_pixels = image::load_from_memory(&photo).unwrap();
    let mut jpeg = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 95)
        .encode_image(&photo_pixels)
        .unwrap();
    let sixteen = image::ImageBuffer::from_fn(256, 128, |x, y| {
        image::Rgb([x as u16 * 257, y as u16 * 513, (x + y) as u16 * 170])
    });
    let mut png16 = Vec::new();
    DynamicImage::ImageRgb16(sixteen)
        .write_to(&mut Cursor::new(&mut png16), ImageFormat::Png)
        .unwrap();
    let mut cases = vec![
        ("ui", include_bytes!("../design/layout-1180.png").to_vec()),
        ("photo", photo),
        ("jpeg", jpeg),
        ("gradient", png(&gradient, None)),
        ("alpha-edge", png(&alpha, None)),
        ("texture", png(&texture, None)),
        (
            "icc",
            include_bytes!("../tests/fixtures/icon-icc.png").to_vec(),
        ),
        ("16-bit", png16),
    ];
    let p3 = std::env::var_os("TINYPNG_P3_PROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/System/Library/ColorSync/Profiles/Display P3.icc"));
    if let Ok(profile) = std::fs::read(p3) {
        cases.push(("display-p3", png(&gradient, Some(&profile))));
    }
    let mut csv = String::from(
        "case,preset,requested_format,actual_format,input_bytes,output_bytes,lossy,score,local_rmse,min_tile_score,tile_accepted,ms\n",
    );
    let mut rows = String::new();
    let mut count = 0;
    for (name, input) in &cases {
        let original = image::load_from_memory(input).unwrap();
        let rgba = original.to_rgba8();
        let source_extension = image::guess_format(input).unwrap().extensions_str()[0];
        let source_name = format!("{name}.source.{source_extension}");
        std::fs::write(directory.join(&source_name), input).unwrap();
        for preset in Preset::ALL {
            for format in [OutputFormat::Keep, OutputFormat::WebP] {
                let options = Options {
                    mode: Mode::Smart,
                    policy: preset.policy(),
                    output_format: format,
                };
                let start = Instant::now();
                let result = squash::squash(input, &options).unwrap();
                let ms = start.elapsed().as_millis();
                let output = image::load_from_memory(&result.bytes).unwrap();
                assert_eq!(output.width(), original.width(), "{name}");
                assert_eq!(output.height(), original.height(), "{name}");
                assert!(result.bytes.len() <= input.len(), "{name} grew");
                let decoded = output.to_rgba8();
                let metrics = quality::measure(&rgba, &decoded).unwrap();
                let tiled = quality::measure_tiled(
                    rgba.width(),
                    rgba.height(),
                    |r| {
                        Ok((
                            image::imageops::crop_imm(&rgba, r.x, r.y, r.width, r.height)
                                .to_image(),
                            image::imageops::crop_imm(&decoded, r.x, r.y, r.width, r.height)
                                .to_image(),
                        ))
                    },
                    |_| true,
                )
                .unwrap();
                assert!(tiled.complete);
                let tile_accepted = options.policy.allows(tiled.metrics);
                // 只对固定语料检查是否出现分块放行、全图拒绝；两个分数不能视为等价。
                assert!(
                    !tile_accepted || options.policy.allows(metrics),
                    "{name} tiled/global disagreement"
                );
                if result.lossy {
                    assert!(
                        options.policy.allows(metrics),
                        "{name} {preset:?} violates quality budget"
                    );
                    assert!(
                        (metrics.score - result.score).abs() < 0.00001,
                        "reported score differs from saved pixels"
                    );
                    assert!(
                        options
                            .policy
                            .cost(result.bytes.len(), input.len(), metrics)
                            < 1.0
                    );
                } else {
                    assert_eq!(
                        original.to_rgba16(),
                        output.to_rgba16(),
                        "{name} lossless pixels changed"
                    );
                }
                if squash::carries_icc(input) {
                    assert!(result.kept_icc, "{name} lost ICC");
                }
                let actual = image::guess_format(&result.bytes).unwrap().extensions_str()[0];
                let requested = if format == OutputFormat::Keep {
                    "keep"
                } else {
                    "webp"
                };
                let output_name = format!("{name}.{}.{requested}.{actual}", preset.key());
                std::fs::write(directory.join(&output_name), &result.bytes).unwrap();
                csv.push_str(&format!(
                    "{name},{},{requested},{actual},{},{},{},{:.5},{:.5},{:.5},{tile_accepted},{ms}\n",
                    preset.key(),
                    input.len(),
                    result.bytes.len(),
                    result.lossy,
                    metrics.score,
                    metrics.local_error,
                    tiled.metrics.score
                ));
                rows.push_str(&format!(
                    "<section data-case='{name}'><h2>{name} · {} · {requested} → {actual}</h2><p>{} → {} B · 全图 {:.2} · 局部 {:.2} · 分块最低 {:.2} / 通过 {tile_accepted} · {ms} ms · {}</p><div class='pair'><figure><figcaption>原图</figcaption><img src='{source_name}'></figure><figure><figcaption>输出</figcaption><img src='{output_name}'></figure></div></section>",
                    preset.key(), input.len(), result.bytes.len(), metrics.score, metrics.local_error, tiled.metrics.score,
                    if result.lossy { "有损" } else { "无损 / 原样" }
                ));
                count += 1;
            }
        }
    }
    let options = cases
        .iter()
        .map(|(name, _)| format!("<option>{name}</option>"))
        .collect::<String>();
    let html = format!(
        r#"<!doctype html><html lang="zh"><meta charset="utf-8"><title>ImgSquash 实图验收</title>
<style>body{{font:14px system-ui;margin:24px;color:#25372e}}nav{{position:sticky;top:0;background:#fff;padding:12px;z-index:1}}button,select{{margin:4px;padding:7px}}section{{border-bottom:1px solid #ddd;padding:12px 0}}.pair{{display:grid;grid-template-columns:1fr 1fr;gap:16px}}figure{{margin:0;overflow:auto;background:var(--bg,repeating-conic-gradient(#eee 0% 25%,#fff 0% 50%) 0 0/16px 16px)}}img{{display:block;max-width:100%;height:auto}}body.actual img{{max-width:none}}figcaption{{padding:8px;background:#eef3eb}}</style>
<h1>实图验收：{count} 个结果</h1><p>覆盖 {cases_count} 类输入；指标验收通过不等于视觉无损。P3 的数值仍按 sRGB 假设，仅验证配置保留；需人工查看颜色。来源：项目 UI、用户提供照片，以及确定生成的边界图。</p>
<nav><select onchange="document.querySelectorAll('section').forEach(s=>s.hidden=this.value!=='全部'&&s.dataset.case!==this.value)"><option>全部</option>{options}</select><button onclick="document.body.classList.toggle('actual')">适应窗口 / 原尺寸 1:1</button><button onclick="document.body.style.setProperty('--bg','#fff')">白底</button><button onclick="document.body.style.setProperty('--bg','#000')">黑底</button><button onclick="document.body.style.removeProperty('--bg')">棋盘底</button><a href="results.csv">CSV</a></nav>{rows}</html>"#,
        cases_count = cases.len()
    );
    std::fs::write(directory.join("results.csv"), csv).unwrap();
    std::fs::write(directory.join("index.html"), html).unwrap();
    println!(
        "acceptance: {} cases, {count} results passed; {}",
        cases.len(),
        directory.display()
    );
}
