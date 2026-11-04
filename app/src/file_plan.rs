//! 有界容器检查与工作计划；先选真实路径，再申请资源。
use crate::{
    jpeg, resources,
    squash::{Mode, Options},
};
use image::ImageFormat;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

pub(crate) const METADATA_LIMIT: usize = 1024 * 1024;
pub(crate) struct Header {
    pub format: ImageFormat,
    pub width: u32,
    pub height: u32,
    pub png: Option<Png>,
    pub jpeg: Option<Jpeg>,
    pub preserve: Option<String>,
    pub source_len: u64,
    modified: Option<std::time::SystemTime>,
    icc: bool,
}
pub(crate) struct Png {
    pub depth: u8,
    pub color: u8,
    pub interlaced: bool,
    pub metadata: Vec<([u8; 4], Vec<u8>)>,
}
pub(crate) struct Jpeg {
    pub info: jpeg::Info,
    pub markers: Vec<(u8, Vec<u8>)>,
    pub progressive: bool,
    pub density: Option<mozjpeg::PixelDensity>,
}
pub(crate) enum Route {
    Original(String),
    Memory { bytes: u64 },
    Stream { bytes: u64, lossy: bool },
}

impl Header {
    pub fn read(source: &mut File) -> Result<Self, String> {
        source.rewind().map_err(|e| e.to_string())?;
        let (width, height) =
            crate::squash::image_dimensions(std::io::BufReader::new(&mut *source))?;
        source.rewind().map_err(|e| e.to_string())?;
        let mut signature = [0; 8];
        source
            .read_exact(&mut signature)
            .map_err(|e| e.to_string())?;
        let source_metadata = source.metadata().map_err(|e| e.to_string())?;
        let mut result = Self {
            format: ImageFormat::Png,
            width,
            height,
            png: None,
            jpeg: None,
            preserve: None,
            source_len: source_metadata.len(),
            modified: source_metadata.modified().ok(),
            icc: false,
        };
        if signature == crate::squash::PNG_SIGNATURE {
            let mut png = Png {
                depth: 8,
                color: 6,
                interlaced: false,
                metadata: Vec::new(),
            };
            let mut metadata = 0;
            let length = source.metadata().map_err(|e| e.to_string())?.len();
            loop {
                let mut head = [0; 8];
                source
                    .read_exact(&mut head)
                    .map_err(|e| format!("PNG 块头损坏: {e}"))?;
                let size = u32::from_be_bytes(head[..4].try_into().unwrap()) as u64;
                let kind: [u8; 4] = head[4..].try_into().unwrap();
                result.icc |= &kind == b"iCCP";
                let end = source
                    .stream_position()
                    .map_err(|e| e.to_string())?
                    .checked_add(size + 4)
                    .filter(|end| *end <= length)
                    .ok_or("PNG 块长度超出文件")?;
                if &kind == b"IHDR" {
                    if size != 13 {
                        return Err("PNG IHDR 长度无效".into());
                    }
                    let mut data = [0; 13];
                    source.read_exact(&mut data).map_err(|e| e.to_string())?;
                    png.depth = data[8];
                    png.color = data[9];
                    png.interlaced = data[12] != 0;
                } else if &kind == b"acTL" {
                    result.preserve = Some("动画 PNG 保留原文件，避免丢帧".into());
                } else if kind[0].is_ascii_lowercase() {
                    metadata += size as usize;
                    if metadata > METADATA_LIMIT {
                        result.preserve = Some("图片辅助信息较大，保留原文件".into());
                    } else {
                        let mut data = vec![0; size as usize];
                        source.read_exact(&mut data).map_err(|e| e.to_string())?;
                        png.metadata.push((kind, data));
                    }
                } else if ![b"PLTE", b"IDAT", b"IEND"].contains(&&kind) {
                    result.preserve = Some("PNG 包含无法安全重编码的关键块，保留原文件".into());
                }
                source
                    .seek(SeekFrom::Start(end))
                    .map_err(|e| e.to_string())?;
                if &kind == b"IEND" {
                    break;
                }
            }
            result.png = Some(png);
        } else {
            result.format = ImageFormat::Jpeg;
            source.seek(SeekFrom::Start(2)).map_err(|e| e.to_string())?;
            let mut bytes = vec![0xff, 0xd8];
            let mut markers = Vec::new();
            let mut progressive = false;
            let mut density = None;
            loop {
                let mut marker = [0; 2];
                source.read_exact(&mut marker).map_err(|e| e.to_string())?;
                if marker[0] != 0xff {
                    return Err("JPEG 标记无效".into());
                }
                while marker[1] == 0xff {
                    source
                        .read_exact(&mut marker[1..])
                        .map_err(|e| e.to_string())?;
                }
                bytes.extend(marker);
                if marker[1] == 0xda {
                    break;
                }
                if matches!(marker[1], 0x01 | 0xd0..=0xd7) {
                    continue;
                }
                let mut size = [0; 2];
                source.read_exact(&mut size).map_err(|e| e.to_string())?;
                let count = u16::from_be_bytes(size) as usize;
                if count < 2 {
                    return Err("JPEG 段长度无效".into());
                }
                if bytes.len() + count > METADATA_LIMIT {
                    result.preserve = Some("JPEG 图片头较大，保留原文件".into());
                    break;
                }
                let mut data = vec![0; count - 2];
                source.read_exact(&mut data).map_err(|e| e.to_string())?;
                bytes.extend(size);
                bytes.extend(&data);
                if marker[1] == 0xe0 && data.starts_with(b"JFIF\0") && data.len() >= 14 {
                    let unit = match data[7] {
                        1 => mozjpeg::PixelDensityUnit::Inches,
                        2 => mozjpeg::PixelDensityUnit::Centimeters,
                        _ => mozjpeg::PixelDensityUnit::PixelAspectRatio,
                    };
                    density = Some(mozjpeg::PixelDensity {
                        unit,
                        x: u16::from_be_bytes([data[8], data[9]]),
                        y: u16::from_be_bytes([data[10], data[11]]),
                    });
                }
                progressive |= matches!(marker[1], 0xc2 | 0xc6 | 0xca | 0xce);
                if matches!(marker[1], 0xe1 | 0xe2 | 0xed | 0xfe) {
                    markers.push((marker[1], data));
                }
            }
            let info = jpeg::inspect(&bytes);
            result.icc = info.has_icc;
            result.jpeg = Some(Jpeg {
                info,
                markers,
                progressive,
                density,
            });
            source.seek(SeekFrom::End(-2)).map_err(|e| e.to_string())?;
            let mut end = [0; 2];
            source.read_exact(&mut end).map_err(|e| e.to_string())?;
            if end != [0xff, 0xd9] {
                result.preserve = Some("JPEG 数据不完整或带有尾随信息，保留原文件".into());
            }
        }
        source.rewind().map_err(|e| e.to_string())?;
        Ok(result)
    }

    pub fn has_icc(&self) -> bool {
        self.icc
    }
    pub fn unchanged(&self, source: &File) -> Result<(), String> {
        let meta = source.metadata().map_err(|e| e.to_string())?;
        if meta.len() != self.source_len || meta.modified().ok() != self.modified {
            Err("图片在处理过程中发生变化，请重新添加后再试".into())
        } else {
            Ok(())
        }
    }

    pub fn route(&self, input: u64, options: &Options, budget: u64) -> Route {
        if let Some(reason) = &self.preserve {
            return Route::Original(reason.clone());
        }
        if let Some(jpeg) = &self.jpeg {
            let info = &jpeg.info;
            if options.mode == Mode::Lossless
                || !matches!(info.components, 1 | 3)
                || info.components == 1 && info.has_icc
                || info.has_icc && info.icc.is_none()
                || input < 2048
                || info.quality.is_some_and(|q| q <= 75)
            {
                return Route::Original("JPEG 无需或不宜再次有损编码，保留原文件".into());
            }
        }
        let lossy = options.mode == Mode::Smart
            && self.width.min(self.height) >= 8
            && !self.png.as_ref().is_some_and(|p| {
                p.depth == 16
                    || p.color == 3
                    || matches!(p.color, 0 | 4) && self.has_icc()
                    || input < 2048 && options.output_format == crate::squash::OutputFormat::Keep
            });
        if input <= resources::MAX_MEMORY_INPUT_BYTES {
            let conversion = options.mode == Mode::Smart
                && options.output_format == crate::squash::OutputFormat::WebP
                && self
                    .png
                    .as_ref()
                    .is_none_or(|p| p.depth != 16 && !(matches!(p.color, 0 | 4) && self.has_icc()));
            if let Ok(bytes) =
                resources::estimate(self.width, self.height, input, lossy || conversion)
                && bytes <= budget
            {
                return Route::Memory { bytes };
            }
        }
        let rows = u64::from(self.width) * 64;
        let coefficients = self
            .jpeg
            .as_ref()
            .filter(|j| j.progressive)
            .map_or(0, |_| u64::from(self.width) * u64::from(self.height) * 16);
        let bytes = (if lossy { 192 } else { 32 }) * 1024 * 1024 + rows + coefficients;
        if bytes <= budget {
            Route::Stream { bytes, lossy }
        } else if self.png.is_some() && 32 * 1024 * 1024 + rows <= budget {
            Route::Stream {
                bytes: 32 * 1024 * 1024 + rows,
                lossy: false,
            }
        } else {
            Route::Original("当前内存不适合安全重编码，保留原尺寸和原文件".into())
        }
    }
}
