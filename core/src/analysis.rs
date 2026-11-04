//! 单次扫描的内容画像与原尺寸采样块；不解析容器，不做编码或业务决策。
use crate::quantize::Palette;
use image::RgbaImage;
use std::{collections::HashMap, sync::Arc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Content {
    Exact,
    Graphic,
    Photo,
    Mixed,
}
#[derive(Debug)]
pub struct Profile {
    pub has_alpha: bool,
    pub grayscale: bool,
    /// None 表示超过 256，避免为照片维护数百万颜色的集合。
    pub unique_colors: Option<usize>,
    pub flat_ratio: f64,
    pub edge_density: f64,
    pub content: Content,
    pub exact: Option<Palette>,
    tiles: Vec<Tile>,
}
#[derive(Debug, Clone, Default)]
struct Tile {
    x: u32,
    y: u32,
    count: u32,
    flat: u32,
    gentle: u32,
    edge: u32,
}

pub struct Analyzed {
    pub pixels: Arc<RgbaImage>,
    pub sample: Arc<RgbaImage>,
    pub profile: Profile,
}
impl Analyzed {
    pub fn new(pixels: RgbaImage) -> Self {
        let profile = Profile::scan(&pixels);
        let pixels = Arc::new(pixels);
        let sample = profile.sample(&pixels);
        Self {
            pixels,
            sample,
            profile,
        }
    }
    /// 按像素量外推样本字节比；仅作采样排名，最终使用实际全图字节数。
    pub fn sample_ratio(&self, sample_bytes: usize, baseline_bytes: usize) -> f64 {
        if baseline_bytes == 0 || self.sample.is_empty() {
            return f64::INFINITY;
        }
        sample_bytes as f64 / baseline_bytes as f64 * self.pixels.len() as f64
            / self.sample.len() as f64
    }
}
impl Profile {
    pub fn scan(image: &RgbaImage) -> Self {
        let (w, h) = image.dimensions();
        let columns = w.div_ceil(128).max(1);
        let mut tiles = vec![Tile::default(); (columns * h.div_ceil(128).max(1)) as usize];
        let mut has_alpha = false;
        let mut grayscale = true;
        let mut colors = Some(HashMap::new());
        let mut entries = Vec::new();
        let mut indices = Vec::with_capacity(image.len() / 4);
        let (mut flat, mut edge) = (0u64, 0u64);
        for (x, y, p) in image.enumerate_pixels() {
            has_alpha |= p[3] != 255;
            grayscale &= p[3] == 0 || (p[0] == p[1] && p[1] == p[2]);
            if let Some(map) = &mut colors {
                let key = p.0;
                let index = if let Some(index) = map.get(&key) {
                    Some(*index)
                } else if entries.len() < 256 {
                    let i = entries.len() as u8;
                    entries.push(key);
                    map.insert(key, i);
                    Some(i)
                } else {
                    None
                };
                if let Some(index) = index {
                    indices.push(index);
                } else {
                    colors = None;
                    entries.clear();
                    indices = Vec::new();
                }
            }
            let mut difference = 0;
            if x > 0 {
                difference = difference.max(visible_difference(p.0, image.get_pixel(x - 1, y).0));
            }
            if y > 0 {
                difference = difference.max(visible_difference(p.0, image.get_pixel(x, y - 1).0));
            }
            // 画像阈值仅作路径启发式，最终质量必须由全图验证确认。
            let is_flat = difference <= 2;
            let is_edge = difference >= 32;
            flat += u64::from(is_flat);
            edge += u64::from(is_edge);
            let tile = &mut tiles[(y / 128 * columns + x / 128) as usize];
            tile.x = x / 128 * 128;
            tile.y = y / 128 * 128;
            tile.count += 1;
            tile.flat += u32::from(is_flat);
            tile.gentle +=
                u32::from(difference > 0 && difference <= 2 && x % 128 != 0 && y % 128 != 0);
            tile.edge += u32::from(is_edge);
        }
        let unique_colors = colors.as_ref().map(HashMap::len);
        let total = (w as f64 * h as f64).max(1.0);
        let flat_ratio = flat as f64 / total;
        let edge_density = edge as f64 / total;
        let content = if unique_colors.is_some() {
            Content::Exact
        } else if flat_ratio > 0.85 && edge_density > 0.01 {
            Content::Graphic
        } else if !has_alpha && flat_ratio < 0.75 && edge_density < 0.30 {
            Content::Photo
        } else {
            Content::Mixed
        };
        let exact = unique_colors.map(|_| {
            let rgb = entries.iter().map(|p| [p[0], p[1], p[2]]).collect();
            let mut alpha: Vec<_> = entries.iter().map(|p| p[3]).collect();
            alpha.truncate(alpha.iter().rposition(|a| *a != 255).map_or(0, |i| i + 1));
            Palette {
                rgb,
                alpha,
                indices,
            }
        });
        Self {
            has_alpha,
            grayscale,
            unique_colors,
            flat_ratio,
            edge_density,
            content,
            exact,
            tiles,
        }
    }
    fn sample(&self, image: &Arc<RgbaImage>) -> Arc<RgbaImage> {
        let (w, h) = image.dimensions();
        if w.max(h) <= 128 || w.min(h) < 8 {
            return Arc::clone(image);
        }
        let tw = w.min(128);
        let th = h.min(128);
        let density = |tile: &Tile, n: u32| n as f64 / tile.count.max(1) as f64;
        let mut selected = Vec::new();
        // 在平坦块中优先覆盖缓慢的非零变化；没有这种块时才退回最平坦块。
        let flat = self
            .tiles
            .iter()
            .filter(|tile| density(tile, tile.flat) >= 0.9)
            .max_by(|a, b| {
                density(a, a.gentle)
                    .total_cmp(&density(b, b.gentle))
                    .then_with(|| density(a, a.flat).total_cmp(&density(b, b.flat)))
            })
            .or_else(|| {
                self.tiles
                    .iter()
                    .max_by(|a, b| density(a, a.flat).total_cmp(&density(b, b.flat)))
            });
        let candidates = [
            flat,
            self.tiles
                .iter()
                .max_by(|a, b| density(a, a.edge).total_cmp(&density(b, b.edge))),
            self.tiles.iter().max_by(|a, b| {
                density(a, a.count - a.flat - a.edge)
                    .total_cmp(&density(b, b.count - b.flat - b.edge))
            }),
        ];
        for tile in candidates.into_iter().flatten() {
            let origin = (tile.x.min(w - tw), tile.y.min(h - th));
            if !selected.contains(&origin) {
                selected.push(origin);
            }
        }
        let sample = RgbaImage::from_fn(tw * selected.len() as u32, th, |x, y| {
            let (ox, oy) = selected[(x / tw) as usize];
            *image.get_pixel(ox + x % tw, oy + y)
        });
        Arc::new(sample)
    }
}
fn visible_difference(a: [u8; 4], b: [u8; 4]) -> u8 {
    if a[3] == 255 && b[3] == 255 {
        return (0..3).map(|c| a[c].abs_diff(b[c])).max().unwrap_or(0);
    }
    let mut error = a[3].abs_diff(b[3]);
    for c in 0..3 {
        let x = (a[c] as u32 * a[3] as u32 + 127) / 255;
        let y = (b[c] as u32 * b[3] as u32 + 127) / 255;
        error = error
            .max(x.abs_diff(y) as u8)
            .max((x + 255 - a[3] as u32).abs_diff(y + 255 - b[3] as u32) as u8);
    }
    error
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;
    #[test]
    fn exact_scan_preserves_hidden_rgb_and_alpha() {
        let image = RgbaImage::from_fn(32, 32, |x, _| {
            Rgba([20, 30, 40, if x < 16 { 0 } else { 90 }])
        });
        let a = Analyzed::new(image.clone());
        assert_eq!(a.profile.unique_colors, Some(2));
        assert!(a.profile.has_alpha);
        assert_eq!(a.profile.exact.unwrap().to_image(32, 32), image);
    }
    #[test]
    fn samples_preserve_native_resolution_and_reuse_small_inputs() {
        let image = RgbaImage::from_fn(1024, 768, |x, y| Rgba([x as u8, y as u8, 0, 255]));
        let a = Analyzed::new(image);
        assert!(a.profile.unique_colors.is_none());
        assert!(a.sample.width() <= 384);
        assert_eq!(a.sample.height(), 128);
        let small = Analyzed::new(RgbaImage::new(16, 16));
        assert!(Arc::ptr_eq(&small.pixels, &small.sample));
    }
    #[test]
    fn a_large_flat_sky_does_not_turn_a_textured_photo_into_ui() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let photo = RgbaImage::from_fn(512, 512, |x, y| {
            if y < 256 {
                return Rgba([70, 120, 180, 255]);
            }
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let noise = ((seed >> 33) % 17) as i32 - 8;
            let c = |v: u32| ((v % 220 + 15) as i32 + noise).clamp(0, 255) as u8;
            Rgba([c(x), c(y), c(x + y), 255])
        });
        assert_eq!(Profile::scan(&photo).content, Content::Photo);
        let ui = RgbaImage::from_fn(512, 512, |x, y| {
            if x % 64 == 0 || y % 64 == 0 {
                Rgba([x as u8, y as u8, 0, 255])
            } else {
                Rgba([250, 250, 250, 255])
            }
        });
        assert_eq!(Profile::scan(&ui).content, Content::Graphic);
    }
    #[test]
    fn flat_sampling_prefers_a_gentle_gradient_over_pure_color() {
        let pixels = RgbaImage::from_fn(512, 128, |x, y| {
            if (256..384).contains(&x) {
                let value = 64 + ((x - 256) / 2) as u8;
                Rgba([value, value, value, 255])
            } else if (128..256).contains(&x) {
                Rgba([if (x + y) % 2 == 0 { 0 } else { 255 }, 0, 0, 255])
            } else {
                Rgba([220, 220, 220, 255])
            }
        });
        let analyzed = Analyzed::new(pixels);
        assert_eq!(analyzed.sample.get_pixel(0, 64)[0], 64);
        assert_eq!(analyzed.sample.get_pixel(126, 64)[0], 127);
        assert!(analyzed.sample.width() <= 384);
        let solid = Analyzed::new(RgbaImage::from_pixel(512, 128, Rgba([80, 90, 100, 255])));
        assert!(solid.sample.pixels().all(|p| p.0 == [80, 90, 100, 255]));
    }
    #[test]
    fn sample_byte_ratio_uses_the_same_full_image_baseline() {
        let analyzed = Analyzed::new(RgbaImage::from_pixel(512, 128, Rgba([80, 90, 100, 255])));
        let projected = 500.0 * analyzed.pixels.len() as f64 / analyzed.sample.len() as f64;
        assert_eq!(analyzed.sample_ratio(500, 2000), projected / 2000.0);
        assert!(analyzed.sample_ratio(500, 0).is_infinite());
    }
}
