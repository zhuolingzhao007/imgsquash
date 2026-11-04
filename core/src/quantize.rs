//! MIT quantizr：原生 RGBA 量化，单图复用直方图和已生成的调色板。
use image::RgbaImage;
use quantizr::{Histogram, Image, Options, QuantizeResult};
use std::collections::{BTreeMap, HashMap};

pub const MAX_COLORS: u32 = 256;
#[derive(Debug, Clone, Copy)]
pub enum Dither {
    None,
    Diffusion,
}

pub struct Prepared<'a> {
    reference: &'a RgbaImage,
    histogram: Histogram,
    results: BTreeMap<u32, QuantizeResult>,
}
impl<'a> Prepared<'a> {
    pub fn new(image: &'a RgbaImage) -> Result<Self, String> {
        if image.width() == 0 || image.height() == 0 {
            return Err("空图片，无法量化".into());
        }
        let reference = image;
        let image = Image::new(
            image.as_raw(),
            image.width() as usize,
            image.height() as usize,
        )
        .map_err(|error| format!("量化输入无效：{error:?}"))?;
        let mut histogram = Histogram::new();
        histogram.add_image(&image);
        Ok(Self {
            reference,
            histogram,
            results: BTreeMap::new(),
        })
    }
    pub fn quantize(&mut self, colors: u32, dither: Dither) -> Result<Palette, String> {
        self.quantize_on(self.reference, colors, dither)
    }
    /// 从全图直方图建立调色板，映射原尺寸采样块；与最终全图共享颜色树结果。
    pub fn quantize_on(
        &mut self,
        target: &RgbaImage,
        colors: u32,
        dither: Dither,
    ) -> Result<Palette, String> {
        let image = Image::new(
            target.as_raw(),
            target.width() as usize,
            target.height() as usize,
        )
        .map_err(|error| format!("映射输入无效：{error:?}"))?;
        let colors = colors.clamp(2, MAX_COLORS);
        let result = self.results.entry(colors).or_insert_with(|| {
            let mut options = Options::default();
            options
                .set_max_colors(colors as i32)
                .expect("颜色数已限制到 2..256");
            QuantizeResult::quantize_histogram(&self.histogram, &options)
        });
        result
            .set_dithering_level(match dither {
                Dither::None => 0.0,
                Dither::Diffusion => 1.0,
            })
            .map_err(|error| format!("抖动配置失败：{error:?}"))?;
        let mut indices = vec![0; image.width * image.height];
        match dither {
            Dither::Diffusion => result
                .remap_image(&image, &mut indices)
                .map_err(|error| format!("调色板映射失败：{error:?}"))?,
            Dither::None => {
                // UI 中重复色值很多，每个色值只查一次最近颜色。
                let mut lookup = HashMap::new();
                const CACHE_COLORS: usize = 65_536;
                for p in image.data.as_chunks::<4>().0 {
                    lookup.entry([p[0], p[1], p[2], p[3]]).or_insert(0u8);
                    if lookup.len() > CACHE_COLORS {
                        break;
                    }
                }
                if lookup.len() > CACHE_COLORS {
                    // 高色数图不为每个颜色再分配 HashMap/排序/拷贝；零抖动可分批精确映射。
                    drop(lookup);
                    for (pixels, output) in image
                        .data
                        .chunks(32_768 * 4)
                        .zip(indices.chunks_mut(32_768))
                    {
                        let chunk = Image::new(pixels, pixels.len() / 4, 1)
                            .map_err(|error| format!("映射输入无效：{error:?}"))?;
                        result
                            .remap_image(&chunk, output)
                            .map_err(|error| format!("调色板映射失败：{error:?}"))?;
                    }
                    let palette = result.get_palette();
                    return Ok(make_palette(
                        palette.entries[..palette.count as usize]
                            .iter()
                            .map(|p| [p.r, p.g, p.b, p.a]),
                        indices,
                    ));
                }
                let mut unique: Vec<_> = lookup.keys().copied().collect();
                unique.sort_unstable();
                let bytes: Vec<_> = unique.iter().flat_map(|p| p.iter().copied()).collect();
                let unique_image = Image::new(&bytes, unique.len(), 1)
                    .map_err(|error| format!("色值映射输入无效：{error:?}"))?;
                let mut mapped = vec![0; unique.len()];
                result
                    .remap_image(&unique_image, &mut mapped)
                    .map_err(|error| format!("色值映射失败：{error:?}"))?;
                for (color, index) in unique.into_iter().zip(mapped) {
                    lookup.insert(color, index);
                }
                for (pixel, index) in image.data.as_chunks::<4>().0.iter().zip(&mut indices) {
                    *index = lookup[&[pixel[0], pixel[1], pixel[2], pixel[3]]];
                }
            }
        }
        let palette = result.get_palette();
        Ok(make_palette(
            palette.entries[..palette.count as usize]
                .iter()
                .map(|p| [p.r, p.g, p.b, p.a]),
            indices,
        ))
    }
}
fn make_palette(colors: impl Iterator<Item = [u8; 4]>, indices: Vec<u8>) -> Palette {
    let mut rgb = Vec::new();
    let mut alpha = Vec::new();
    for p in colors {
        rgb.push([p[0], p[1], p[2]]);
        alpha.push(p[3]);
    }
    alpha.truncate(alpha.iter().rposition(|a| *a != 255).map_or(0, |i| i + 1));
    Palette {
        rgb,
        alpha,
        indices,
    }
}

#[derive(Debug, Clone)]
pub struct Palette {
    /// 调色板 RGB 条目。
    pub rgb: Vec<[u8; 3]>,
    /// 按调色板顺序的 alpha。可能短于 `rgb`：尾部全不透明的条目无需存储，
    /// 按 PNG 规范缺失的 tRNS 条目按 255 处理。全不透明时为空。
    pub alpha: Vec<u8>,
    /// 每像素一个索引，行主序。
    pub indices: Vec<u8>,
}

impl Palette {
    /// 是否真的需要写 tRNS：存在任何一个非不透明条目。
    pub fn has_transparency(&self) -> bool {
        self.alpha.iter().any(|value| *value != 255)
    }

    pub fn colors(&self) -> usize {
        self.rgb.len()
    }

    /// 展开成 RGBA 图：索引 PNG 解码回来就是调色板查表，所以算质量指标时
    /// 直接展开即可，不必先编码成 PNG 再解码。
    pub fn to_image(&self, width: u32, height: u32) -> RgbaImage {
        let mut image = RgbaImage::new(width, height);
        for (pixel, index) in image.pixels_mut().zip(self.indices.iter()) {
            let slot = *index as usize;
            debug_assert!(slot < self.rgb.len(), "调色板索引越界：{slot}");
            let color = self.rgb.get(slot).copied().unwrap_or([0, 0, 0]);
            // alpha 表会被截断掉尾部的不透明条目，缺的按 PNG 规范算 255。
            let alpha = self.alpha.get(slot).copied().unwrap_or(255);
            *pixel = image::Rgba([color[0], color[1], color[2], alpha]);
        }
        image
    }

    /// 编码成 8 位索引 PNG。调色板、tRNS、索引数据都由量化结果直接给出，
    /// 不再走一遍真彩色编码，这也是索引 PNG 体积小的根本原因。
    ///
    /// 固定写 8 位：颜色数少于 16 时理论上 4 位更小，但这一步的输出还要过 oxipng，
    /// 它的位深缩减（`bit_depth_reduction`）会处理；不依赖 oxipng 的场景下才会漏。
    pub fn to_png(&self, width: u32, height: u32) -> Result<Vec<u8>, String> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_compression(png::Compression::Fast);
            encoder.set_color(png::ColorType::Indexed);
            encoder.set_depth(png::BitDepth::Eight);

            let mut palette_bytes = Vec::with_capacity(self.rgb.len() * 3);
            for color in &self.rgb {
                palette_bytes.extend_from_slice(color);
            }
            encoder.set_palette(palette_bytes);
            if self.has_transparency() {
                encoder.set_trns(self.alpha.clone());
            }

            let mut writer = encoder
                .write_header()
                .map_err(|error| format!("PNG 编码失败: {error}"))?;
            writer
                .write_image_data(&self.indices)
                .map_err(|error| format!("PNG 写入失败: {error}"))?;
            writer
                .finish()
                .map_err(|error| format!("PNG 收尾失败: {error}"))?;
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounded_color_cache_matches_direct_zero_dither_mapping() {
        let image = RgbaImage::from_fn(512, 256, |x, y| {
            image::Rgba([x as u8, y as u8, (x / 256) as u8, 255])
        });
        let mut prepared = Prepared::new(&image).unwrap();
        let actual = prepared.quantize_on(&image, 128, Dither::None).unwrap();
        let result = prepared.results.get_mut(&128).unwrap();
        let input = Image::new(image.as_raw(), 512, 256).unwrap();
        let mut expected = vec![0; 512 * 256];
        result.remap_image(&input, &mut expected).unwrap();
        assert_eq!(actual.indices, expected);
    }
    use image::Rgba;
    #[test]
    fn exact_palette_preserves_all_alpha_and_hidden_rgb() {
        let image = RgbaImage::from_fn(32, 32, |x, y| {
            Rgba([30, 120, 220, [0, 90, 255][((x + y) % 3) as usize]])
        });
        let palette = crate::analysis::Profile::scan(&image).exact.unwrap();
        assert_eq!(palette.colors(), 3);
        assert_eq!(palette.to_image(32, 32), image);
        let bytes = palette.to_png(32, 32).unwrap();
        assert_eq!(image::load_from_memory(&bytes).unwrap().to_rgba8(), image);
    }
    #[test]
    fn independent_runs_reproduce_complex_rgba_palettes() {
        let image = RgbaImage::from_fn(72, 64, |x, y| {
            Rgba([(x * 3) as u8, (y * 4) as u8, 100, (x * 3) as u8])
        });
        for dither in [Dither::None, Dither::Diffusion] {
            let a = Prepared::new(&image).unwrap().quantize(64, dither).unwrap();
            for _ in 0..3 {
                let b = Prepared::new(&image).unwrap().quantize(64, dither).unwrap();
                assert_eq!(a.rgb, b.rgb);
                assert_eq!(a.alpha, b.alpha);
                assert_eq!(a.indices, b.indices);
            }
        }
    }

    #[test]
    fn complex_images_use_bounded_valid_indices() {
        let image = RgbaImage::from_fn(32, 32, |x, y| Rgba([x as u8 * 8, y as u8 * 8, 100, 255]));
        let mut prepared = Prepared::new(&image).unwrap();
        for colors in [32, 128, 64] {
            let palette = prepared.quantize(colors, Dither::None).unwrap();
            assert_eq!(palette.indices.len(), 1024);
            assert!(palette.colors() <= colors as usize);
            assert!(palette
                .indices
                .iter()
                .all(|i| (*i as usize) < palette.colors()));
        }
        assert!(Prepared::new(&RgbaImage::new(0, 0)).is_err());
    }
    #[test]
    fn sampling_reuses_the_global_palette_and_maps_every_target_pixel() {
        let full = RgbaImage::from_fn(64, 64, |x, y| {
            Rgba([(x * 4) as u8, (y * 4) as u8, 100, 255])
        });
        let sample = image::imageops::crop_imm(&full, 16, 16, 32, 32).to_image();
        let mut prepared = Prepared::new(&full).unwrap();
        let sampled = prepared.quantize_on(&sample, 64, Dither::None).unwrap();
        let final_palette = prepared.quantize(64, Dither::None).unwrap();
        assert_eq!(sampled.rgb, final_palette.rgb);
        assert_eq!(sampled.alpha, final_palette.alpha);
        let expanded = final_palette.to_image(64, 64);
        assert_eq!(
            sampled.to_image(32, 32),
            image::imageops::crop_imm(&expanded, 16, 16, 32, 32).to_image()
        );
    }
}
