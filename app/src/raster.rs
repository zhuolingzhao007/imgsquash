//! 磁盘像素缓冲。内存只保存当前行或读取区域，文件关闭自动删除。
use image::RgbaImage;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
};
use tinypng_core::quality::Region;

pub(crate) struct Raster {
    file: File,
    pub width: u32,
    pub height: u32,
    pub color: png::ColorType,
    pub depth: png::BitDepth,
    pub palette: Option<Vec<u8>>,
    pub trns: Option<Vec<u8>>,
    stride: usize,
}
impl Raster {
    pub fn new(
        width: u32,
        height: u32,
        color: png::ColorType,
        depth: png::BitDepth,
    ) -> Result<Self, String> {
        let bits = color.samples() * depth as usize;
        let stride = (u64::from(width) * bits as u64).div_ceil(8);
        let length = stride
            .checked_mul(u64::from(height))
            .ok_or("像素临时文件过大")?;
        let stride = usize::try_from(stride).map_err(|_| "图片行过宽")?;
        let file = tempfile::tempfile().map_err(|e| format!("创建像素临时文件失败: {e}"))?;
        file.set_len(length)
            .map_err(|e| format!("无法预留像素临时文件: {e}"))?;
        Ok(Self {
            file,
            width,
            height,
            color,
            depth,
            palette: None,
            trns: None,
            stride,
        })
    }
    pub fn stride(&self) -> usize {
        self.stride
    }
    pub fn row(&mut self, y: u32, buffer: &mut [u8]) -> Result<(), String> {
        if y >= self.height || buffer.len() != self.stride {
            return Err("像素行读取越界".into());
        }
        self.file
            .seek(SeekFrom::Start(u64::from(y) * self.stride as u64))
            .map_err(|e| e.to_string())?;
        self.file
            .read_exact(buffer)
            .map_err(|e| format!("读取像素临时文件失败: {e}"))
    }
    pub fn write_row(&mut self, y: u32, buffer: &[u8]) -> Result<(), String> {
        if y >= self.height || buffer.len() != self.stride {
            return Err("像素行写入越界".into());
        }
        self.file
            .seek(SeekFrom::Start(u64::from(y) * self.stride as u64))
            .map_err(|e| e.to_string())?;
        self.file
            .write_all(buffer)
            .map_err(|e| format!("写入像素临时文件失败: {e}"))
    }
    pub fn region(&mut self, r: Region) -> Result<RgbaImage, String> {
        if self.color != png::ColorType::Rgba
            || self.depth != png::BitDepth::Eight
            || r.x.checked_add(r.width).is_none_or(|end| end > self.width)
            || r.y
                .checked_add(r.height)
                .is_none_or(|end| end > self.height)
        {
            return Err("RGBA 区域读取越界或格式不符".into());
        }
        let mut image = RgbaImage::new(r.width, r.height);
        for (i, row) in image
            .as_mut()
            .chunks_exact_mut(r.width as usize * 4)
            .enumerate()
        {
            let offset = u64::from(r.y + i as u32) * self.stride as u64 + u64::from(r.x) * 4;
            self.file
                .seek(SeekFrom::Start(offset))
                .map_err(|e| e.to_string())?;
            self.file.read_exact(row).map_err(|e| e.to_string())?;
        }
        Ok(image)
    }
    /// 确定性分层抽样，颜色学习最多 262144 个像素；不为大图建无限颜色直方图。
    pub fn histogram_sample(&mut self) -> Result<RgbaImage, String> {
        let count = u64::from(self.width) * u64::from(self.height);
        let step = count.div_ceil(262_144).max(1);
        let mut data = Vec::new();
        let mut row = vec![0; self.stride];
        let mut next = 0u64;
        for y in 0..self.height {
            let start = u64::from(y) * u64::from(self.width);
            self.row(y, &mut row)?;
            while next < start + u64::from(self.width) && next < count {
                let index = (next - start) as usize * 4;
                data.extend_from_slice(&row[index..index + 4]);
                // 每个区间取一个位置，轻微抖动避免固定周期纹理与步长混叠。
                let bucket = next / step + 1;
                next = bucket
                    .checked_mul(step)
                    .and_then(|base| {
                        base.checked_add(bucket.wrapping_mul(0x9e3779b97f4a7c15) % step)
                    })
                    .unwrap_or(count);
            }
        }
        RgbaImage::from_raw((data.len() / 4) as u32, 1, data).ok_or_else(|| "颜色样本为空".into())
    }
    pub fn sample(&mut self) -> Result<RgbaImage, String> {
        let w = self.width.min(128);
        let h = self.height.min(128);
        let locations = [
            (0, 0),
            ((self.width - w) / 2, (self.height - h) / 2),
            (self.width - w, self.height - h),
        ];
        let mut sample = RgbaImage::new(w * 3, h);
        for (i, (x, y)) in locations.into_iter().enumerate() {
            let tile = self.region(Region {
                x,
                y,
                width: w,
                height: h,
            })?;
            image::imageops::replace(&mut sample, &tile, i as i64 * i64::from(w), 0);
        }
        Ok(sample)
    }
}
