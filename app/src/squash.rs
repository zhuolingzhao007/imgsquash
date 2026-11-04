//! 压缩后端的总入口：判定输入格式，分派给 `png` / `jpeg`，两者都可能先走
//! `convert` 做格式转换。
//!
//! 编码器生成候选，tinypng-core 提供测量/选择策略，本层处理文件边界与安全落盘。

use crate::{jpeg, png};
use image::{ImageFormat, RgbaImage};
use std::io::{Cursor, Read, Seek, SeekFrom};
use std::path::Path;
use tinypng_core::png_meta;
use tinypng_core::policy::{Policy, Preset};
use tinypng_core::quality::Metrics;

pub(crate) const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
pub(crate) const JPEG_SIGNATURE: &[u8] = &[0xFF, 0xD8, 0xFF];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Smart,
    Lossless,
}

impl Mode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "smart" => Ok(Mode::Smart),
            "lossless" => Ok(Mode::Lossless),
            other => Err(format!("未知的压缩模式：{other}")),
        }
    }
}

/// 输出格式。`Keep` 表示保持原格式，不做转换。
///
/// 只有 WebP：AVIF 在 `image` 里没有解码器，无法回读验证，所以不提供输出。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputFormat {
    Keep,
    WebP,
}

impl OutputFormat {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "keep" => Ok(OutputFormat::Keep),
            "webp" => Ok(OutputFormat::WebP),
            other => Err(format!("未知的输出格式：{other}")),
        }
    }

    /// 界面上显示的名字。
    pub fn label(self) -> &'static str {
        match self {
            OutputFormat::Keep => "原格式",
            OutputFormat::WebP => "WebP",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Options {
    pub mode: Mode,
    pub policy: Policy,
    pub output_format: OutputFormat,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            mode: Mode::Smart,
            policy: Preset::default().policy(),
            output_format: OutputFormat::Keep,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Outcome {
    pub bytes: Vec<u8>,
    /// 输出文件的扩展名。`None` 表示沿用调用方给的路径（没转换格式）。
    pub extension: Option<String>,
    pub method: String,
    pub score: f64,
    pub err: f64,
    pub lossy: bool,
    /// 输出文件里**确实**带着 ICC 色彩配置。由字节流判定，不由调用方传。
    pub kept_icc: bool,
    /// 给界面看的一句话说明（为什么没转换、为什么退回无损等）。
    pub hint: Option<String>,
}

impl Outcome {
    pub(crate) fn with_hint(mut self, hint: Option<String>) -> Self {
        if hint.is_some() {
            self.hint = hint;
        }
        self
    }
}

#[derive(Debug, Clone)]
pub struct FileOutcome {
    pub input_size: u64,
    pub output_size: u64,
    /// 实际写出的路径。格式转换会改扩展名，所以它可能和调用方传的 `output` 不同。
    pub output_path: String,
    pub method: String,
    pub score: f64,
    pub err: f64,
    pub lossy: bool,
    pub kept_icc: bool,
    pub hint: Option<String>,
    /// global / tiled：分块最低分不冒充整图分数。
    pub score_kind: String,
}

/// 一个经过实际编码与测量的候选。
pub(crate) struct Candidate<T = Vec<u8>> {
    pub bytes: T,
    pub method: String,
    pub metrics: Metrics,
    pub lossy: bool,
}

/// 压缩一个文件：读 → 压 → 写同目录临时文件 → 不覆盖地发布。
///
/// 输出路径以 `Outcome.extension` 为准：选了 WebP 但收益不足时，写出的仍然是
/// PNG，扩展名必须跟着实际字节走，不能留着 `.webp` 里装 PNG。
pub fn compress_file(
    input: &Path,
    output: &Path,
    options: &Options,
) -> Result<FileOutcome, String> {
    compress_file_with_budget(input, output, options, crate::resources::shared())
}

pub(crate) fn compress_file_with_budget(
    input: &Path,
    output: &Path,
    options: &Options,
    budget: &crate::resources::Budget,
) -> Result<FileOutcome, String> {
    if same_file(input, output) {
        return Err("输入和输出是同一个文件".to_string());
    }
    let mut source =
        std::fs::File::open(input).map_err(|error| format!("无法读取输入文件: {error}"))?;
    let header = crate::file_plan::Header::read(&mut source)?;
    let length = header.source_len;
    let dimensions = (header.width, header.height);
    let estimate = match header.route(length, options, budget.limit()) {
        crate::file_plan::Route::Memory { bytes } => bytes,
        route => {
            let ext = if header.png.is_some() {
                "png"
            } else if input
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("jpeg"))
            {
                "jpeg"
            } else {
                "jpg"
            };
            let target = output.with_extension(ext);
            if same_file(input, &target) {
                return Err("实际输出和输入是同一个文件".into());
            }
            return match route {
                crate::file_plan::Route::Original(reason) => {
                    let _reservation = budget.acquire(1024 * 1024)?;
                    crate::streaming::original(&mut source, &header, length, &target, reason)
                }
                crate::file_plan::Route::Stream { bytes, lossy } => {
                    let _reservation = budget.acquire(bytes)?;
                    match crate::streaming::run(
                        &mut source,
                        &header,
                        length,
                        &target,
                        options,
                        lossy,
                    ) {
                        Err(error) if error.starts_with("低内存资源限制") => {
                            crate::streaming::original(
                                &mut source,
                                &header,
                                length,
                                &target,
                                format!("{error}；保留原文件和原尺寸"),
                            )
                        }
                        result => result,
                    }
                }
                _ => unreachable!(),
            };
        }
    };
    let _reservation = budget.acquire(estimate)?;
    source
        .seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    let mut original = Vec::new();
    original
        .try_reserve_exact(length as usize)
        .map_err(|_| "无法分配输入缓冲，请减少并发或缩小图片".to_string())?;
    (&mut source)
        .take(crate::resources::MAX_MEMORY_INPUT_BYTES + 1)
        .read_to_end(&mut original)
        .map_err(|error| format!("无法读取输入文件: {error}"))?;
    if original.len() as u64 != length || image_dimensions(Cursor::new(&original))? != dimensions {
        return Err("图片在读取过程中发生变化，请重新添加后再试".into());
    }
    let outcome = squash(&original, options)?;
    header.unchanged(&source)?;

    let source_ext = input.extension().and_then(|s| s.to_str()).unwrap_or("");
    let original_ext = if original.starts_with(PNG_SIGNATURE) {
        "png"
    } else if source_ext.eq_ignore_ascii_case("jpeg") {
        "jpeg"
    } else {
        "jpg"
    };
    let target = output.with_extension(outcome.extension.as_deref().unwrap_or(original_ext));
    // 必须检查实际落盘路径：转换/回退后它可能已不是调用方传来的 output。
    if same_file(input, &target) {
        return Err("实际输出和输入是同一个文件".into());
    }
    let target = write_atomic(&target, &outcome.bytes)?;

    Ok(FileOutcome {
        input_size: original.len() as u64,
        output_size: outcome.bytes.len() as u64,
        output_path: target.to_string_lossy().into_owned(),
        method: outcome.method,
        score: outcome.score,
        err: outcome.err,
        lossy: outcome.lossy,
        kept_icc: outcome.kept_icc,
        hint: outcome.hint,
        score_kind: "global".into(),
    })
}

pub(crate) fn image_dimensions(
    mut reader: impl std::io::BufRead + Seek,
) -> Result<(u32, u32), String> {
    let mut signature = [0u8; 8];
    reader
        .read_exact(&mut signature)
        .map_err(|error| format!("无法读取图片头: {error}"))?;
    reader
        .seek(SeekFrom::Start(0))
        .map_err(|error| error.to_string())?;
    if signature == PNG_SIGNATURE {
        // 只读 IHDR；不能为读尺寸先解析/解压整份 ICC 或像素数据。
        let mut decoder =
            ::png::Decoder::new_with_limits(reader, ::png::Limits { bytes: 1024 * 1024 });
        let info = decoder
            .read_header_info()
            .map_err(|error| format!("无法读取 PNG 尺寸: {error}"))?;
        return Ok((info.width, info.height));
    }
    if signature.starts_with(JPEG_SIGNATURE) {
        return jpeg::dimensions(reader).map_err(|error| format!("无法读取 JPEG 尺寸: {error}"));
    }
    Err("只支持 PNG 和 JPEG".into())
}

pub fn squash(input: &[u8], options: &Options) -> Result<Outcome, String> {
    if input.starts_with(PNG_SIGNATURE) {
        png::squash(input, options)
    } else if input.starts_with(JPEG_SIGNATURE) {
        jpeg::squash(input, options)
    } else {
        Err("只支持 PNG 和 JPEG".to_string())
    }
}

/// 压缩率，0–100。两条 API 以前一个截断一个四舍五入，同一张图会差 1%。
pub fn ratio(before: u64, after: u64) -> u32 {
    if before == 0 {
        return 0;
    }
    let saved = (before as f64 - after as f64) / before as f64 * 100.0;
    saved.round().clamp(0.0, 100.0) as u32
}

/// 收益够不够。不够就原样输出，绝不把图压大。
pub(crate) fn is_profitable(before: usize, after: usize, min_gain: f64) -> bool {
    after > 0 && after < before && (after as f64) <= before as f64 * (1.0 - min_gain)
}

/// 没有任何改动：把原始字节原样带回去。
pub(crate) fn unchanged(input: &[u8], method: &str) -> Outcome {
    Outcome {
        bytes: input.to_vec(),
        extension: None,
        method: method.to_string(),
        score: 100.0,
        err: 0.0,
        lossy: false,
        kept_icc: carries_icc(input),
        hint: None,
    }
}

/// 与实际无损基线比较；没有抵偿画质损失的收益，就保留基线。
pub(crate) fn choose(baseline: Outcome, candidate: Candidate, options: &Options) -> Outcome {
    if !options.policy.allows(candidate.metrics)
        || !is_profitable(
            baseline.bytes.len(),
            candidate.bytes.len(),
            if candidate.lossy {
                options.policy.limits().min_gain
            } else {
                0.0
            },
        )
        || options.policy.cost(
            candidate.bytes.len(),
            baseline.bytes.len(),
            candidate.metrics,
        ) >= 1.0
    {
        return baseline;
    }
    Outcome {
        kept_icc: carries_icc(&candidate.bytes),
        bytes: candidate.bytes,
        extension: None,
        method: candidate.method,
        score: candidate.metrics.score,
        err: candidate.metrics.err,
        lossy: candidate.lossy,
        hint: None,
    }
}

/// 选了目标格式但这条路走不了时给界面一句解释；没选就什么都不说。
pub(crate) fn format_skipped(options: &Options, reason: &str) -> Option<String> {
    if options.output_format == OutputFormat::Keep {
        return None;
    }
    Some(format!(
        "未转换为 {}：{reason}",
        options.output_format.label()
    ))
}

/// 字节流里有没有 ICC。`kept_icc` 一律从这里来，不再靠调用方传标志位——
/// 之前那个标志位在「原样输出」「转换成功」两条路径上都在说谎。
pub(crate) fn carries_icc(bytes: &[u8]) -> bool {
    if bytes.starts_with(PNG_SIGNATURE) {
        png_meta::has_iccp(bytes)
    } else if bytes.starts_with(JPEG_SIGNATURE) {
        jpeg::has_icc(bytes)
    } else {
        // WebP 交给 libwebp 的绑定解析 RIFF/VP8X，不再手写容器解析。
        webpx::get_icc_profile(bytes).ok().flatten().is_some()
    }
}

pub(crate) fn decode(bytes: &[u8], format: ImageFormat) -> Result<RgbaImage, String> {
    image::load_from_memory_with_format(bytes, format)
        .map(|image| image.to_rgba8())
        .map_err(|error| format!("{format:?} 解码失败: {error}"))
}

/// 同一个文件的不同写法（`./a.png`、符号链接、大小写不敏感文件系统上的 `A.png`）
/// 都要认出来，否则临时文件 rename 会把原图覆盖掉。
fn same_file(input: &Path, output: &Path) -> bool {
    let (Ok(input), Ok(output)) = (std::fs::canonicalize(input), std::fs::canonicalize(output))
    else {
        // 输出还不存在，那就不可能和输入是同一个文件。
        return false;
    };
    input == output
}

/// 临时文件和目标在同目录。发布采用 no-clobber，路径检查后发生的并发撞名
/// 也不会覆盖已有文件；每次操作有独立临时文件，失败由 RAII 清理。
fn write_atomic(path: &Path, bytes: &[u8]) -> Result<std::path::PathBuf, String> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    std::fs::create_dir_all(parent).map_err(|error| format!("无法创建输出目录: {error}"))?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("创建临时文件失败: {error}"))?;
    temp.write_all(bytes)
        .map_err(|error| format!("写入输出文件失败: {error}"))?;
    publish(temp, path)
}

/// 内存结果和磁盘流式结果使用同一不覆盖发布机制。
pub(crate) fn publish(
    mut temp: tempfile::NamedTempFile,
    path: &Path,
) -> Result<std::path::PathBuf, String> {
    temp.as_file()
        .sync_all()
        .map_err(|error| format!("同步输出文件失败: {error}"))?;
    let stem = path.file_stem().unwrap_or_default();
    for suffix in 1..u32::MAX {
        let target = if suffix == 1 {
            path.to_path_buf()
        } else {
            let mut name = stem.to_os_string();
            name.push(format!("-{suffix}"));
            if let Some(extension) = path.extension() {
                name.push(".");
                name.push(extension);
            }
            path.with_file_name(name)
        };
        match temp.persist_noclobber(&target) {
            Ok(_) => return Ok(target),
            Err(error) if error.error.kind() == std::io::ErrorKind::AlreadyExists => {
                temp = error.file
            }
            Err(error) => return Err(format!("保存输出失败: {}", error.error)),
        }
    }
    Err("无法分配输出文件名".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn output_format_round_trips() {
        for format in ["keep", "webp"] {
            assert!(OutputFormat::parse(format).is_ok());
        }
        // AVIF 没有解码器，无法回读验证，所以不支持。
        assert!(OutputFormat::parse("avif").is_err());
        assert!(OutputFormat::parse("gif").is_err());
    }

    #[test]
    fn ratio_rounds_instead_of_truncating() {
        // 1000 → 875 是 12.5%，应该进到 13 而不是截成 12。
        assert_eq!(ratio(1000, 875), 13);
        assert_eq!(ratio(1000, 1000), 0);
        assert_eq!(ratio(0, 0), 0);
        // 压大了也不能出现负数（u32 会回绕成天文数字）。
        assert_eq!(ratio(1000, 1200), 0);
    }

    #[test]
    fn profitability_needs_the_minimum_gain() {
        assert!(is_profitable(1000, 900, 0.03));
        assert!(!is_profitable(1000, 980, 0.03));
        assert!(!is_profitable(1000, 1000, 0.03));
        assert!(!is_profitable(1000, 1200, 0.03));
    }

    #[test]
    fn unchanged_reports_the_icc_it_actually_carries() {
        let fixture = std::fs::read("tests/fixtures/icon-icc.png").unwrap();
        assert!(unchanged(&fixture, "original").kept_icc);
        let plain = std::fs::read("tests/fixtures/icon.png").unwrap();
        assert!(!unchanged(&plain, "original").kept_icc);
    }

    #[test]
    fn format_skipped_only_speaks_when_a_format_was_chosen() {
        let mut options = Options::default();
        assert_eq!(format_skipped(&options, "16 位 PNG"), None);
        options.output_format = OutputFormat::WebP;
        assert!(
            format_skipped(&options, "16 位 PNG")
                .unwrap()
                .contains("WebP")
        );
    }

    #[test]
    fn rejects_unknown_input() {
        let error = squash(b"GIF89a", &Options::default()).unwrap_err();
        assert!(error.contains("PNG 和 JPEG"), "{error}");
    }

    #[test]
    fn file_boundary_streams_large_inputs_instead_of_reading_them_into_memory() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("huge.png");
        let output = directory.path().join("output.png");
        std::fs::write(&input, include_bytes!("../tests/fixtures/icon.png")).unwrap();
        std::fs::OpenOptions::new()
            .write(true)
            .open(&input)
            .unwrap()
            .set_len(crate::resources::MAX_MEMORY_INPUT_BYTES + 1)
            .unwrap();
        let result = compress_file(&input, &output, &Options::default()).unwrap();
        assert_eq!(
            result.input_size,
            crate::resources::MAX_MEMORY_INPUT_BYTES + 1
        );
        assert!(result.output_size < 2048);
        assert_eq!(
            image::open(result.output_path).unwrap().to_rgba8(),
            image::open(input).unwrap().to_rgba8()
        );
    }

    #[test]
    fn tiny_png_with_huge_dimensions_is_rejected_before_decoding() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("huge-pixels.png");
        let output = directory.path().join("output.png");
        let mut bytes = Vec::new();
        {
            let mut encoder = ::png::Encoder::new(&mut bytes, 1_000_000, 1_000_000);
            encoder.set_color(::png::ColorType::Rgba);
            let _writer = encoder.write_header().unwrap();
        }
        assert!(bytes.len() < 1024);
        assert_eq!(
            image_dimensions(Cursor::new(&bytes)).unwrap(),
            (1_000_000, 1_000_000)
        );
        std::fs::write(&input, bytes).unwrap();
        let error = compress_file(&input, &output, &Options::default()).unwrap_err();
        assert!(error.contains("PNG"), "{error}");
        assert!(!output.exists());
    }

    #[test]
    fn failed_conversion_restores_the_original_extension() {
        // 界面按目标格式占了 photo.webp，转换被拒后退回 PNG——扩展名必须改回来。
        let dir = std::env::temp_dir().join(format!("tinypng-fallback-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let input = dir.join("photo.png");
        let bytes = std::fs::read("tests/fixtures/icon.png").unwrap();
        std::fs::write(&input, &bytes).unwrap();
        let reserved = dir.join("compressed/photo.webp");

        let options = Options {
            output_format: OutputFormat::WebP,
            // 无损模式不转换，验证实际输出恢复原格式。
            mode: Mode::Lossless,
            ..Options::default()
        };

        let outcome = compress_file(&input, &reserved, &options).unwrap();
        assert!(
            outcome.output_path.ends_with("photo.png"),
            "{}",
            outcome.output_path
        );
        assert!(!reserved.exists(), "不该在 .webp 路径上留下 PNG 字节");
        assert!(std::path::Path::new(&outcome.output_path).exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unique_path_steps_around_existing_files() {
        let dir = std::env::temp_dir().join(format!("tinypng-unique-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = dir.join("photo.png");
        std::fs::write(&first, b"x").unwrap();
        assert_eq!(
            write_atomic(&first, b"two").unwrap(),
            dir.join("photo-2.png")
        );
        std::fs::write(dir.join("photo-2.png"), b"x").unwrap();
        assert_eq!(
            write_atomic(&first, b"three").unwrap(),
            dir.join("photo-3.png")
        );
        assert_eq!(std::fs::read(first).unwrap(), b"x");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn final_extension_cannot_turn_output_back_into_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("icon.png");
        let bytes = std::fs::read("tests/fixtures/icon.png").unwrap();
        std::fs::write(&input, &bytes).unwrap();
        let options = Options {
            mode: Mode::Lossless,
            output_format: OutputFormat::WebP,
            ..Options::default()
        };
        assert!(compress_file(&input, &dir.path().join("icon.webp"), &options).is_err());
        assert_eq!(std::fs::read(input).unwrap(), bytes);
    }

    #[test]
    fn concurrent_writers_never_overwrite_each_other_or_existing_files() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("same.png");
        std::fs::write(&target, b"original").unwrap();
        let threads: Vec<_> = (0..4u8)
            .map(|i| {
                let target = target.clone();
                std::thread::spawn(move || (i, write_atomic(&target, &[i; 4096]).unwrap()))
            })
            .collect();
        let mut paths = std::collections::BTreeSet::new();
        for thread in threads {
            let (value, path) = thread.join().unwrap();
            assert_eq!(std::fs::read(&path).unwrap(), vec![value; 4096]);
            assert!(paths.insert(path));
        }
        assert_eq!(std::fs::read(target).unwrap(), b"original");
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            5,
            "无残留临时文件"
        );
    }

    /// 真实图基准：`TINYPNG_BENCH=xxx.png cargo test --lib bench_real_screenshot -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_real_screenshot() {
        let path = std::env::var("TINYPNG_BENCH").expect("设置 TINYPNG_BENCH 指向一张 PNG");
        let bytes = std::fs::read(path).unwrap();
        let started = std::time::Instant::now();
        let outcome = squash(&bytes, &Options::default()).unwrap();
        if let Some(path) = std::env::var_os("TINYPNG_BENCH_OUT") {
            std::fs::write(path, &outcome.bytes).unwrap();
        }
        println!(
            "{} -> {} bytes, {:?}, {}, score {:.2}, err {:.0}, lossy {}, icc {}, hint {:?}",
            bytes.len(),
            outcome.bytes.len(),
            started.elapsed(),
            outcome.method,
            outcome.score,
            outcome.err,
            outcome.lossy,
            outcome.kept_icc,
            outcome.hint
        );
    }

    /// 格式对比：同一张图在「保持原格式」和「转 WebP」下各档的收益。
    ///
    /// `TINYPNG_BENCH=xxx.png cargo test --lib bench_formats -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn bench_formats() {
        let path = std::env::var("TINYPNG_BENCH").expect("设置 TINYPNG_BENCH 指向一张 PNG");
        let bytes = std::fs::read(path).unwrap();
        println!("源文件 {} 字节", bytes.len());
        for preset in Preset::ALL {
            for format in [OutputFormat::Keep, OutputFormat::WebP] {
                let options = Options {
                    policy: preset.policy(),
                    output_format: format,
                    ..Options::default()
                };
                let started = std::time::Instant::now();
                let outcome = squash(&bytes, &options).unwrap();
                println!(
                    "  {:>8} {:<5} -> {:>8} 字节 ({:>5.1}%) {:>7.2?} {:<10} 感知 {:>5.1} 误差 {:>3.0}{}",
                    preset.key(),
                    format.label(),
                    outcome.bytes.len(),
                    outcome.bytes.len() as f64 / bytes.len() as f64 * 100.0,
                    started.elapsed(),
                    outcome.method,
                    outcome.score,
                    outcome.err,
                    outcome
                        .hint
                        .map(|hint| format!("  hint: {hint}"))
                        .unwrap_or_default()
                );
            }
        }
    }
}
