//! The app's Rust code: what `App.tsx` imports from `tinypng-mac/native`. Their
//! TypeScript declarations are generated from these types into
//! `.nsh/types/tinypng-mac/native.d.ts`.
//!
//! 跨平台约定：所有和系统打交道的部分（文件面板、文件管理器、路径拼接）
//! 都按 `#[cfg]` 分平台实现，上层 TSX 只调用统一的模块接口。

use nsh_core::prelude::*;
use std::path::Path;
use tinypng_core::policy::Preset;

#[cfg(target_os = "macos")]
use objc2::rc::{Retained, autoreleasepool};
#[cfg(target_os = "macos")]
use objc2_app_kit::NSOpenPanel;
#[cfg(target_os = "macos")]
use objc2_foundation::{NSArray, NSString, NSURL};

mod convert;
mod file_plan;
mod jpeg;
mod pipeline;
mod png;
mod raster;
mod resources;
mod squash;
mod stream_jpeg;
mod stream_png;
mod streaming;
mod timing;

#[cfg(test)]
mod acceptance;
#[cfg(test)]
mod large_tests;

/// 智能压缩参数。
///
/// `preset` 仅传画质偏好名称，由核心策略统一解释。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, JsObject)]
pub struct AutoArgs {
    pub input: String,
    pub output: String,
    /// "smart" 或 "lossless"
    pub mode: String,
    /// "standard" | "strict" | "smaller"
    pub preset: String,
    /// "keep" | "webp"
    pub output_format: String,
}

/// 智能压缩结果：带上实际用的方法、SSIMULACRA2 和误差，界面可以直接展示。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, JsObject)]
pub struct AutoResult {
    pub input_size: u64,
    pub output_size: u64,
    /// 实际写出的路径。格式转换会改扩展名，所以它可能和传进来的 `output` 不同。
    pub output_path: String,
    pub ratio: u32,
    pub method: String,
    pub score: f64,
    pub score_kind: String,
    pub err: f64,
    pub lossy: bool,
    /// 输出文件里确实带着原图的 ICC 色彩配置。
    pub kept_icc: bool,
    /// 压缩后的提示文案（为什么没转换、为什么退回无损等），界面在 FileRow 中展示。
    pub hint: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, JsObject)]
pub struct ImageInfo {
    pub width: u32,
    pub height: u32,
}

/// 宏模块：`tinypng-mac/native` 下的函数，TSX 通过 `from "tinypng-mac/native"` 导入。
#[native_module("tinypng-mac/native")]
fn native(registry: &mut Registry) {
    registry
        .module::<Compress>()
        .module::<FilePicker>()
        .module::<Finder>()
        .module::<Paths>();
}

/// 图片压缩模块：`Compress.auto({ input, output, mode, preset, outputFormat })`。
#[module]
pub struct Compress;

#[module]
impl Compress {
    /// 只读图片头，不解码像素。导入时用它避免给超大图加载全尺寸缩略图。
    pub async fn inspect(input: String) -> Result<ImageInfo, String> {
        nsh_core::element::ambient::blocking(move || {
            let file = std::fs::File::open(input).map_err(|error| error.to_string())?;
            let (width, height) = squash::image_dimensions(std::io::BufReader::new(file))?;
            Ok(ImageInfo { width, height })
        })
        .await
        .map_err(|error| error.to_string())?
    }

    /// 可以同时跑几张：按 CPU 核数，最多 4。
    /// 每张图解码后要占几十 MB，核多的机器也不放开。
    pub fn parallelism() -> u32 {
        let cores = std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(2);
        cores.clamp(1, 4) as u32
    }

    /// 智能压缩：按实际体积与感知/局部损失选择；不足以抵偿失真时保留无损基线。
    pub async fn auto(args: AutoArgs) -> Result<AutoResult, String> {
        let input = std::path::PathBuf::from(&args.input);
        let output = std::path::PathBuf::from(&args.output);
        let options = squash::Options {
            mode: squash::Mode::parse(&args.mode)?,
            policy: Preset::parse(&args.preset)?.policy(),
            output_format: squash::OutputFormat::parse(&args.output_format)?,
        };

        let outcome = nsh_core::element::ambient::blocking(move || {
            squash::compress_file(&input, &output, &options)
        })
        .await
        .map_err(|error| format!("压缩任务失败: {error}"))??;

        Ok(AutoResult {
            input_size: outcome.input_size,
            output_size: outcome.output_size,
            ratio: squash::ratio(outcome.input_size, outcome.output_size),
            output_path: outcome.output_path,
            method: outcome.method,
            score: outcome.score,
            score_kind: outcome.score_kind,
            err: outcome.err,
            lossy: outcome.lossy,
            kept_icc: outcome.kept_icc,
            hint: outcome.hint,
        })
    }
}

/// 文件选择模块：`FilePicker.pickFiles()` 调系统自带的文件面板。
/// 图片和文件夹使用独立面板，避免类型过滤影响文件夹选择。
#[module]
pub struct FilePicker;

#[module]
impl FilePicker {
    /// 弹出文件选择面板，可多选图片，返回选中的路径列表。
    pub async fn pick_files() -> Result<Vec<String>, String> {
        // 面板是阻塞式的（macOS 上是模态循环），放到阻塞线程池上等，
        // 别占住异步执行器——否则整个界面会卡在面板上。
        nsh_core::element::ambient::blocking(|| pick_paths_blocking(false))
            .await
            .map_err(|error| format!("文件选择面板失败: {error}"))?
    }

    /// 多选文件夹，交给队列已有的递归扫描逻辑。
    pub async fn pick_folders() -> Result<Vec<String>, String> {
        nsh_core::element::ambient::blocking(|| pick_paths_blocking(true))
            .await
            .map_err(|error| format!("文件夹选择面板失败: {error}"))?
    }

    /// 把拖入的 URI 解析成本地路径。Finder 拖入的是 file:// 路径；从别的
    /// 应用（浏览器、Electron、Xcode）拖图时，macOS 给的是文件引用 URL
    /// （file:///.file/id=…），要靠 NSURL.filePathURL 才能解析回真实路径。
    /// Windows 上则是把 file:///C:/… 还原成 C:\…。
    /// 解析不了的原样返回，交给上层报错。
    pub fn resolve_uris(uris: Vec<String>) -> Vec<String> {
        uris.into_iter().map(|uri| resolve_uri(&uri)).collect()
    }
}

/// 路径工具：界面层不自己拼路径。分隔符、盘符、`file://` 编码都交给这里，
/// 免得 TSX 里出现 `split("/")` 这种只在 macOS 上成立的写法。
#[module]
pub struct Paths;

#[module]
impl Paths {
    /// 相对路径按进程工作目录展开；原生路径与文件权限检查使用同一绝对路径。
    pub fn absolute(path: String) -> Result<String, String> {
        std::path::absolute(path)
            .map(|path| path.to_string_lossy().into_owned())
            .map_err(|error| error.to_string())
    }

    /// 拼一段路径：`Paths.join("/a/b", "c.png")` → `/a/b/c.png`，
    /// Windows 上是 `C:\a\b\c.png`。
    pub fn join(base: String, name: String) -> String {
        Path::new(&base).join(name).to_string_lossy().to_string()
    }

    /// 父目录：`/a/b/c.png` → `/a/b`。没有父目录时返回空串。
    pub fn parent(path: String) -> String {
        Path::new(&path)
            .parent()
            .map(|parent| parent.to_string_lossy().to_string())
            .unwrap_or_default()
    }

    /// 最后一段：`/a/b/c.png` → `c.png`；取不到时原样返回。
    pub fn base(path: String) -> String {
        Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone())
    }

    /// 换扩展名：`/a/b/pic.png` + `webp` → `/a/b/pic.webp`。
    /// 输出路径由 TSX 先占位，扩展名要跟着目标格式走，两边口径必须一致。
    pub fn with_extension(path: String, extension: String) -> String {
        Path::new(&path)
            .with_extension(extension)
            .to_string_lossy()
            .to_string()
    }

    /// 转成 `file://` URL。`<Image src>` 只认 URL / `asset:`，Windows 的
    /// `C:\…` 里的冒号会被当成 scheme，所以缩略图要走这个。
    pub fn to_file_url(path: String) -> String {
        let text = path.replace('\\', "/");
        // UNC：\\server\share → file://server/share
        if let Some(rest) = text.strip_prefix("//") {
            return format!("file://{}", percent_encode(rest));
        }
        let text = if text.starts_with('/') {
            text
        } else {
            format!("/{text}")
        };
        format!("file://{}", percent_encode(&text))
    }
}

/// Finder / 资源管理器辅助：在文件管理器里显示输出文件，或打开文件夹。
#[module]
pub struct Finder;

#[module]
impl Finder {
    /// 在文件管理器中选中并显示 `path`。
    pub fn reveal(path: String) -> Result<(), String> {
        reveal(&path)
    }

    /// 在文件管理器中打开文件夹 `path`。
    pub fn open_folder(path: String) -> Result<(), String> {
        open_folder(&path)
    }
}

// ---------------------------------------------------------------------------
// 文件选择面板：每个平台一份
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn pick_paths_blocking(folders: bool) -> Result<Vec<String>, String> {
    dispatch2::run_on_main(|mtm| {
        autoreleasepool(|_| {
            let panel: Retained<NSOpenPanel> = NSOpenPanel::openPanel(mtm);

            panel.setCanChooseFiles(!folders);
            panel.setCanChooseDirectories(folders);
            panel.setAllowsMultipleSelection(true);
            // 扩展名过滤够用，避免为 UTType 再引一个依赖。
            #[allow(deprecated)]
            {
                if folders {
                    panel.setAllowedFileTypes(None);
                } else {
                    panel.setAllowedFileTypes(Some(&NSArray::from_retained_slice(&[
                        NSString::from_str("png"),
                        NSString::from_str("jpg"),
                        NSString::from_str("jpeg"),
                    ])));
                }
            }
            panel.setTitle(Some(&NSString::from_str(if folders {
                "添加文件夹"
            } else {
                "添加图片"
            })));
            panel.setPrompt(Some(&NSString::from_str("添加")));

            // 模态选择，返回 1 表示点了「打开」。
            let response: isize = panel.runModal();
            if response != 1 {
                // 1 == NSFileHandlingPanelOKButton
                return Ok(vec![]);
            }

            let urls: Retained<NSArray<NSURL>> = panel.URLs();
            let mut result = Vec::with_capacity(urls.len());
            for url in urls.iter() {
                let nsurl: &NSURL = &url;
                if let Some(path_buf) = nsurl.to_file_path() {
                    result.push(path_buf.to_string_lossy().to_string());
                }
            }
            Ok(result)
        })
    })
}

#[cfg(target_os = "windows")]
fn pick_paths_blocking(folders: bool) -> Result<Vec<String>, String> {
    let panel = rfd::FileDialog::new();
    let paths = if folders {
        panel.set_title("添加文件夹").pick_folders()
    } else {
        panel
            .add_filter("图片", &["png", "jpg", "jpeg"])
            .pick_files()
    };
    Ok(paths
        .unwrap_or_default()
        .into_iter()
        .map(|path| path.to_string_lossy().to_string())
        .collect())
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn pick_paths_blocking(_folders: bool) -> Result<Vec<String>, String> {
    Err("当前平台还没有文件选择面板，请把图片直接拖进窗口".to_string())
}

// ---------------------------------------------------------------------------
// 拖入 URI → 本地路径
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn resolve_uri(uri: &str) -> String {
    if !uri.starts_with("file:") {
        return uri.to_owned();
    }
    let text = NSString::from_str(uri);
    NSURL::URLWithString(&text)
        .and_then(|url| url.filePathURL())
        .and_then(|url| url.to_file_path())
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| uri.to_string())
}

#[cfg(not(target_os = "macos"))]
fn resolve_uri(uri: &str) -> String {
    file_url_to_path(uri).unwrap_or_else(|| uri.to_string())
}

/// `file:///C:/a%20b.png` → `C:\a b.png`；`file://server/share/a.png` → `\\server\share\a.png`。
#[cfg(target_os = "windows")]
fn file_url_to_path(uri: &str) -> Option<String> {
    let rest = uri.strip_prefix("file://")?;
    let decoded = percent_decode(rest);
    if let Some(path) = decoded.strip_prefix('/') {
        // file:///C:/…：盘符前那个斜杠是 URL 的，不是路径的。
        let path = path.replace('/', "\\");
        return (!path.is_empty()).then_some(path);
    }
    if decoded.is_empty() {
        return None;
    }
    // file://host/share/…：网络路径。
    Some(format!("\\\\{}", decoded.replace('/', "\\")))
}

/// `file:///a/b.png` → `/a/b.png`（Linux 等 POSIX 平台）。
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn file_url_to_path(uri: &str) -> Option<String> {
    Some(percent_decode(uri.strip_prefix("file://")?))
}

#[cfg(not(target_os = "macos"))]
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' && index + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[index + 1..index + 3]).ok();
            if let Some(byte) = hex.and_then(|hex| u8::from_str_radix(hex, 16).ok()) {
                out.push(byte);
                index += 3;
                continue;
            }
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

/// URL 路径段的百分号编码：保留非保留字符和 `/`、`:`（盘符要留着）。
fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// 在文件管理器中显示 / 打开文件夹
// ---------------------------------------------------------------------------

#[cfg(target_os = "macos")]
fn reveal(path: &str) -> Result<(), String> {
    let status = std::process::Command::new("open")
        .args(["-R", path])
        .status()
        .map_err(|error| format!("无法打开 Finder: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("Finder 未能打开该文件".to_string())
    }
}

#[cfg(target_os = "windows")]
fn reveal(path: &str) -> Result<(), String> {
    // explorer 成功时也常返回非 0 退出码，所以只看能不能启动，不看退出码。
    std::process::Command::new("explorer")
        .arg(format!("/select,{path}"))
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开资源管理器: {error}"))
}

#[cfg(target_os = "linux")]
fn reveal(path: &str) -> Result<(), String> {
    open_folder(
        &Path::new(path)
            .parent()
            .map(|parent| parent.to_string_lossy().to_string())
            .unwrap_or_default(),
    )
}

#[cfg(target_os = "macos")]
fn open_folder(path: &str) -> Result<(), String> {
    let status = std::process::Command::new("open")
        .arg(path)
        .status()
        .map_err(|error| format!("无法打开文件夹: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("Finder 未能打开该文件夹".to_string())
    }
}

#[cfg(target_os = "windows")]
fn open_folder(path: &str) -> Result<(), String> {
    std::process::Command::new("explorer")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("无法打开资源管理器: {error}"))
}

#[cfg(target_os = "linux")]
fn open_folder(path: &str) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(path)
        .status()
        .map_err(|error| format!("无法打开文件夹: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> std::path::PathBuf {
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("tinypng-{tag}-{id}"));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn icon() -> image::RgbaImage {
        image::RgbaImage::from_fn(256, 256, |x, y| {
            let dx = x as i32 - 128;
            let dy = y as i32 - 128;
            if dx * dx + dy * dy < 90 * 90 {
                image::Rgba([30, 120, 220, 255])
            } else {
                image::Rgba([0, 0, 0, 0])
            }
        })
    }

    #[test]
    fn auto_compresses_and_reports_the_written_path() {
        let dir = temp_dir("auto");
        let input = dir.join("icon.png");
        let output = dir.join("compressed/icon.png");
        icon().save(&input).unwrap();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let result = runtime
            .block_on(Compress::auto(AutoArgs {
                input: input.to_string_lossy().to_string(),
                output: output.to_string_lossy().to_string(),
                mode: "smart".to_string(),
                preset: "standard".to_string(),
                output_format: "keep".to_string(),
            }))
            .unwrap();

        assert!(result.output_size < result.input_size);
        assert!(!result.lossy, "精确调色板是无损结果");
        assert_eq!(result.output_path, output.to_string_lossy());
        assert!(output.exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn converted_output_gets_the_matching_extension() {
        let dir = temp_dir("webp");
        let input = dir.join("photo.png");
        let output = dir.join("compressed/photo.png");
        // 用渐变图：索引 PNG 压不动，转 WebP 才有收益。
        let gradient = image::RgbaImage::from_fn(256, 256, |x, y| {
            image::Rgba([x as u8, y as u8, ((x + y) / 2) as u8, 255])
        });
        gradient.save(&input).unwrap();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let result = runtime
            .block_on(Compress::auto(AutoArgs {
                input: input.to_string_lossy().to_string(),
                output: output.to_string_lossy().to_string(),
                mode: "smart".to_string(),
                preset: "standard".to_string(),
                output_format: "webp".to_string(),
            }))
            .unwrap();

        assert!(
            result.method.starts_with("webp"),
            "method={} hint={:?}",
            result.method,
            result.hint
        );
        assert!(
            result.output_path.ends_with("photo.webp"),
            "{}",
            result.output_path
        );
        assert!(
            !std::path::Path::new(&output).exists(),
            "不该在 .png 路径上留下 WebP 字节"
        );
        let bytes = std::fs::read(&result.output_path).unwrap();
        assert_eq!(&bytes[0..4], b"RIFF", "写出的应是 WebP 容器");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unknown_preset_is_rejected() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let error = runtime
            .block_on(Compress::auto(AutoArgs {
                input: "/nope.png".to_string(),
                output: "/nope-out.png".to_string(),
                mode: "smart".to_string(),
                preset: "ultra".to_string(),
                output_format: "keep".to_string(),
            }))
            .unwrap_err();
        assert!(error.contains("档位"), "{error}");
    }

    #[test]
    fn lossless_never_grows_and_keeps_pixels() {
        let dir = temp_dir("lossless");
        let input = dir.join("alpha.png");
        let output = dir.join("optimized.png");
        let mut image = image::RgbaImage::new(64, 64);
        for (x, y, pixel) in image.enumerate_pixels_mut() {
            *pixel = image::Rgba([x as u8 * 4, y as u8 * 4, 128, (x + y) as u8 * 2]);
        }
        image.save(&input).unwrap();
        let original_bytes = std::fs::read(&input).unwrap();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let result = runtime
            .block_on(Compress::auto(AutoArgs {
                input: input.to_string_lossy().to_string(),
                output: output.to_string_lossy().to_string(),
                mode: "lossless".to_string(),
                preset: "standard".to_string(),
                output_format: "keep".to_string(),
            }))
            .unwrap();

        assert_eq!(image::open(&output).unwrap().to_rgba8(), image);
        assert_eq!(std::fs::read(&input).unwrap(), original_bytes);
        assert!(result.output_size <= result.input_size);
        assert_eq!(
            result.output_size,
            std::fs::metadata(&output).unwrap().len()
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn paths_join_and_split_like_the_host() {
        let base = if cfg!(target_os = "windows") {
            "C:\\pics"
        } else {
            "/pics"
        };
        let joined = Paths::join(base.to_string(), "compressed".to_string());
        assert!(joined.ends_with("compressed"));
        assert_eq!(Paths::base(joined.clone()), "compressed");
        assert_eq!(Paths::parent(joined.clone()), base);
        assert_eq!(Paths::base("/a/b/icon.png".to_string()), "icon.png");
    }

    #[test]
    fn with_extension_swaps_only_the_suffix() {
        let path = if cfg!(target_os = "windows") {
            "C:\\pics\\a.png"
        } else {
            "/pics/a.png"
        };
        let swapped = Paths::with_extension(path.to_string(), "webp".to_string());
        assert!(swapped.ends_with(".webp"), "{swapped}");
        assert!(swapped.contains("a."), "{swapped}");
    }

    #[test]
    fn file_urls_round_trip() {
        let path = if cfg!(target_os = "windows") {
            "C:\\pics\\a b.png".to_string()
        } else {
            "/pics/a b.png".to_string()
        };
        let url = Paths::to_file_url(path.clone());
        assert!(url.starts_with("file://"), "{url}");
        assert!(!url.contains(' '), "空格要编码：{url}");
        // macOS 走 NSURL 解析，其余平台走自带的 file:// 解析。
        assert_eq!(resolve_uri(&url), path);
    }
}
