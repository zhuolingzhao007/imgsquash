> 历史评审记录。文中的固定门闸和 zenquant/zenwebp 方案已被 2026-10-27 的 [核心流程改造](../app/docs/plans/2026-10-27-adaptive-compression.md) 替代，当前行为以新文档和测试为准。

# tinypng / ImgSquash 代码评审

评审范围：`core/src/*`（561 行）、`app/src/*`（1484 行），以及与之耦合的  
`state/compression.ts`、`state/settings.ts`、`components/FileRow.tsx`、`components/Settings.tsx`。

工具结果：`cargo clippy --all-targets` 在 `tinypng-mac` 上零告警；`tinypng-core` 只有 1 条  
`manual_checked_ops`（`quantize.rs:201`）。也就是说**下面的问题都不是 lint 能扫出来的**，  
全部是逻辑、契约和图像处理层面的问题。

---

## 总评

分层是对的：`tinypng-core` 是纯函数式压缩内核（量化 / 指标 / ICC），`squash.rs` 是编排，  
`lib.rs` 只做 FFI 和平台适配，`state/` 按职责切分。质量闸门「感知分 + 分位误差」双闸门、  
alpha 预乘、`best_candidate` 真编码比体积这几处，是同类工具里少见的讲究。

问题集中在两块：

1. **格式转换（WebP/AVIF）是后来补上去的支路**，它绕开了主流程的几个不变量，导致 5 个  
   用户可见的错误，而且**零测试覆盖**。
2. **`policy.rs` 和 `state/settings.ts` 各存了一份阈值**，`policy.rs` 的注释却声称  
   「避免两处判定不一致」——注释与事实相反。

---

## P0 会产生错误结果

### 1. 输出文件扩展名不跟随目标格式

`state/compression.ts:37` 用原文件名生成输出路径：

```ts
const candidate = Paths.join(directory, candidateName(file.name, suffix));
```

`candidateName` 只改<u>主干、保留扩展名，</u>所以 `p`**`ic.png` +&#x20;**&#x9009; WebP → 磁盘上是  
`compressed/pic.png`，内容却是 WebP 字节流。Rust 侧 `compress_file` 也不改扩展名。

后果：Finder 双击用预览打不开、拖进浏览器可能被当 PNG 解、上传到只认后缀的服务直接报错。

修法：把「格式 → 扩展名」的映射放在 Rust 侧（`Outcome` 增加 `suggested_ext`），  
或 TS 侧在 `reserveOutput` 前按 `outputFormat()` 换掉后缀。**别两边各写一份。**

### 2. `method` 里硬编码了 `webp`

`squash.rs:468`：

```rust
method: format!("{}-webp", current_outcome.method),
```

选 AVIF 时 `method` 是 `zenquant-c128-webp`，`queue.ts:68` 的 `methodLabel` 会把它渲染成  
「128 色 → WEBP」，界面上直接显示错格式。

### 3. `kept_icc` 在三处说谎

| 位置                            | 情况                                   | 实际                                 |
| ----------------------------- | ------------------------------------ | ---------------------------------- |
| `squash.rs:484` `unchanged()` | APNG / CMYK JPEG / 无损模式 JPEG 原样输出    | 恒返回 `false`，但文件原封不动带着 ICC          |
| `squash.rs:472` 转换分支          | `kept_icc: current_outcome.kept_icc` | WebP/AVIF 编码器**根本不写 ICC**，标志位却说保住了 |
| `squash.rs:464`               | `icc::ensure_iccp(&buf, None)`       | 传 `None` 是空操作，注释却写「贴回 ICC（若有）」     |

第 2、3 条合起来是「用户以为色彩配置保住了，实际丢了」。宽色域图（Display P3 截图）转  
WebP 后颜色会明显偏，而这正是这个工具最需要守住的地方。

### 4. 转换失败时的提示文不对题，且真实原因被丢弃

`squash.rs:209-216`：

```rust
Err(_e) => {
    let mut outcome = outcome;
    outcome.hint = Some("照片类 PNG 建议用 WebP 格式输出，可再减小体积 40-60%".to_string());
    return Ok(outcome);
}
```

用户已经选了 WebP，转换没过闸门，提示却建议「用 WebP 输出」。而且 `_e` 把  
「感知 62.3（阈值 78）/ 体积收益不足 / 编码失败」这些真实原因全丢了，用户只能看到一个  
自相矛盾的建议。

### 5. `keep` 模式下超过 200 KB 就提示转 WebP

`squash.rs:221`：

```rust
if options.output_format == "keep" && input.len() > 200 * 1024 {
```

判据是字节数，跟「是不是照片」毫无关系。带 alpha 的图标、UI 截图、大尺寸插画全部中招，  
而且 WebP 对带 alpha 的图标通常并不比索引 PNG 小。这里还会覆盖前面任何 hint。

建议判据换成「颜色数 > 阈值 且 无透明 且 有损压缩率低于 X%」，或者干脆只在实际测过  
一次转换收益后才提示。

### 6. 「无损」模式和 JPEG 输入会静默忽略格式选择

- `squash.rs:154-167`：`Mode::Lossless` 和 16 位 PNG 走 `lossless_png` 后**直接 return**，  
  后面的转换分支永远到不了。
- `squash.rs:232-274`：`squash_jpeg` 里根本没有 `output_format` 的处理。

所以「无损 + WebP」得到 PNG，「JPEG 输入 + WebP」得到 JPEG，界面上的选项是个摆设。  
要么在 UI 上禁用非法组合，要么在 Rust 里显式实现。

### 7. `ratio` 两条路径两种舍入口径

- `lib.rs:97`：`(x * 100.0) as u32` —— 截断
- `lib.rs:142`：`(x * 100.0).round() as u32` —— 四舍五入

同一张图走 `Compress.png` 和 `Compress.auto` 会差 1%。抽一个 `fn ratio(before, after) -> u32`  
放进 core。

### 8. `input == output` 的检查挡不住真覆盖

`squash.rs:106` 只比字面 `Path`。`./a.png` vs `a.png`、符号链接、大小写不敏感文件系统  
（APFS 默认）都能绕过去，然后 `rename` 会把原图覆盖掉。应该比 `canonicalize()` 或 inode。  
目前输出固定落在 `compressed/` 子目录，风险低，但这是「用户数据」级别的问题，值得一行代码解决。

---

## P1 设计与契约

### 9. `policy.rs` 的阈值和 `state/settings.ts` 各存一份

`policy.rs:1` 写着：

> app 的压缩流程和界面档位都从这里取，避免两处判定不一致。

实际 `state/settings.ts:9` 硬编码了 78/28、88/16、70/36，每次都通过 `AutoArgs` 传下来覆盖  
`Options::default()`。`DEFAULT_MIN_SCORE / DEFAULT_MAX_ERR / DEFAULT_MIN_GAIN` 只被  
`Options::default()` 用到，而真实调用永远走 `..Options::default()` 之外的分支。

这是最典型的「注释描述的架构和代码不是一回事」。两条路选一条：

- Rust 导出一个 `presets() -> Vec<Preset>` 原生函数，TS 只负责渲染（推荐，阈值只有一处）；
- 或者承认 UI 是权威，把 `policy.rs` 里那三个 `DEFAULT_*` 删掉，只留 `JPEG_Q_MIN/MAX`。

### 10. 五个结构体字段互相抄

`Options`(8 字段) / `Outcome`(7) / `FileOutcome`(8) / `AutoArgs`(7) / `AutoResult`(9)  
大量重复。`FileOutcome` 严格等于 `Outcome` + `input_size`；`AutoResult` 等于 `FileOutcome`

- `output_path` + `ratio`。每加一个字段要改 4 处，`kept_icc` 这类漏改就是这么来的。

建议：

```rust
pub struct FileOutcome { pub input_size: u64, #[serde(flatten)] pub outcome: Outcome }
```

或者用 `#[derive(From)]` 之类，让 FFI 层只做 `impl From<FileOutcome> for AutoResult`。

### 11. `output_format: String` —— 字符串类型的枚举

`Mode` 有 `parse()` 做了校验，`output_format` 没有。`squash.rs:425` 的  
`_ => return Err("不支持的输出格式")` 把类型错误推到了运行期，而且这个 Err 还会被  
`squash_png` 的 `Err(_e)` 吞掉变成一句错误的提示。

```rust
pub enum OutputFormat { Keep, WebP, Avif }
```

顺手能解决第 2 条（`method` 里的格式名可以直接 `format.name()`）。

### 12. `max_err_convert` 是个假的「可配置」

`Options.max_err_convert` 存在，但 `AutoArgs` 里**没有这个字段**，UI 永远传不进来。  
结果：「严格」档把 PNG 闸门收紧到 `maxErr: 16`，转换路径却仍然用默认的 30——  
用户选了严格，转出来的 WebP 反而比 PNG 更松。

要么加进 `AutoArgs`，要么删掉字段、把 30 写死在转换函数里，别假装可配。

### 13. 死 API：`Compress.png` + `CompressArgs.quality`

`grep Compress.png` 在 TSX 里零命中，只有 Rust 单测在用。`quality` 字段全程没读过  
（`lib.rs:79` 注释自己说「为兼容旧调用而保留」）。而且 `Compress::png` 是**同步**的，  
大图 oxipng 会直接卡住调用线程——`auto()` 特意用了 `ambient::blocking`，`png()` 没有。

没有外部调用方，就直接删掉，别留一条会卡 UI 的兼容路径。

### 14. `ZENQUANT_VERSION` 常量无人引用

`quantize.rs:18` 定义了一个字符串常量，注释写「与 Cargo.toml 保持一致」——手工同步的常量  
必然会漂移，而且没有任何代码读它。删掉。

### 15. `// Fix 2:` / `// Fix 3:` 是补丁编号残留

`squash.rs:199、220、414`。读者不知道 Fix 1 是什么，也不知道还有没有 Fix 4。  
这类注释应该改写成正常的设计说明（「从原图转换，避免双重有损」本身就是好理由，  
不需要编号）。

### 16. `Paths.to_file_url` 是死代码，而缩略图正在裸用路径

`lib.rs:219` 的注释明确说「`<Image src>` 只认 URL / `asset:`，Windows 的 `C:\…` 会被当成  
scheme，所以缩略图要走这个」。但 `components/FileRow.tsx:15` 是：

```tsx
<Image src={f.path} ... />
```

TSX 里对 `toFileUrl` 零调用。macOS 上恰好能用，Windows 上缩略图会挂——而 CI 里有 Windows  
构建任务，说明这个平台是在支持范围内的。要么用上这个 helper，要么删掉它并承认不支持。

### 17. macOS 文件面板在 async 函数里跑模态循环

`lib.rs:256-290`：`async fn pick_files()` 里 `dispatch2::run_on_main(|mtm| ... panel.runModal())`。  
如果 nsh 的异步执行器就在主线程上（GUI 框架通常如此），这就是主线程等主线程的死锁形状；  
即使不死锁，整个异步执行器也会被模态循环堵住。

标准做法是 `beginWithCompletionHandler` + `oneshot` channel，把 `await` 真正让出去。

---

## P2 性能

### 18. 选转换格式时，前面整条 PNG 量化链白跑

`squash.rs:178-217` 的顺序是：4 档 zenquant 量化 → 4 次 SSIMULACRA2 → 编码比体积 →  
oxipng，**然后**才判断 `output_format != "keep"` 并调用 `convert_format_from_bytes`。  
而那个函数只用了 `current_outcome.method` 和 `.kept_icc` 两个字段，字节流整个丢弃。

按 README 自己给的基准（2360×1600 截图约 7.2 s），选 WebP 的每一张图都白烧 7 秒。

修法：把 `output_format` 的判断提到函数开头，转换分支直接解码 → 编码 → 验闸门，不量化。

### 19. `visible_error` 的分配量

`quality.rs:135-147`：4 次 `flatten_into` + 4 次 `buf.clone()`，每份 `3·W·H` 字节。  
4 MP 图 = 约 48 MB 的临时分配，每个候选一次，四档就是 200 MB churn。

黑底和白底可以一次遍历同时累加两个直方图，alpha 直方图也一样，一份缓冲都不用留。  
预期能省掉这里 90% 的分配。

### 20. `score_over` 每次转两遍 `Vec<[f32;3]>`

`quality.rs:87-91`：带 alpha 时黑底白底各转一遍，每遍 12 字节/像素。4 MP 就是 48 MB × 2。  
可以复用一个 `Vec<[f32;3]>`，`clear()` 后重填。

### 21. `best_candidate` 比的是 oxipng **之前**的体积

`squash.rs:178-181` 用 `probe.encoded.len()` 择优，但最终只有胜者会跑 oxipng，而 oxipng  
对索引 PNG 的收益恰恰跟调色板结构相关（不同档位能省的字节数不一样）。所以「挑最小的」  
可能挑错。

要么四档都跑 oxipng（贵一倍以上），要么把这个近似写进注释，别让读者以为这是精确最优。

### 22. 依赖重复编译

`cargo tree -d` 实测：

- `tinypng-core` 同时编了 `png 0.17.16`（自己声明的）和 `png 0.18.1`（`image 0.25` 内部用的）
- `tinypng-mac` 同时有 `image 0.24.9`（nsh 拉的）和 `image 0.25.10`
- AVIF 通过 `ravif → rav1e` 引入了整个 AV1 编码器，是构建时间里最重的一块

`png` 升到 0.18 可以去掉一份。AVIF 建议做成可选 feature——只用 PNG/JPEG 的用户不该为  
rav1e 付编译和体积成本。

---

## P3 图像处理专业性

### 23. WebP 走的是**无损**编码器，闸门和文案都是假的

`image 0.25.10/src/codecs/webp/encoder.rs` 的文档自己写着：

> Right now only **lossless** encoding is supported.

所以 `squash.rs:443` 算出来的 `err` 恒为 0、`score` 恒为 100，`max_err_convert` 这道闸门  
对 WebP 完全不起作用。而 `components/Settings.tsx:88` 和 `squash.rs:213/224` 都在承诺  
「再减小 40-60%」——无损 WebP 相对索引 PNG 通常只有 20-30%，图标类更低。

要真做有损 WebP，纯 Rust 侧有 `zenwebp`（Imazen，和 zenquant 同一家，AGPL-3.0 或商业授权，
编解码都支持），不必引 libwebp 绑定；接上之后像 JPEG 那样按闸门搜质量即可。  
在做到之前，至少把文案改成「无损 WebP」，别把有损的预期挂在无损编码上。

### 24. AVIF 固定 q80，没有质量搜索

`image 0.25.10/src/codecs/avif/encoder.rs:59`：

```rust
AvifEncoder::new_with_speed_quality(w, 4, 80) // `cavif` uses these defaults
```

`DynamicImage::write_to(Avif)` 走的就是这个默认值。所以 JPEG 那条「二分找过闸门的最小质量」  
的精心设计，AVIF 完全没享受到：选「严格」档，AVIF 还是 q80。

应该用 `AvifEncoder::new_with_speed_quality` 显式二分，和 JPEG 一致。

### 25. JPEG 重编码不管色度抽样

`jpeg_candidate`（`squash.rs:327-331`）设了 quality / progressive / optimize_scans，但没设  
抽样方式。mozjpeg 会按质量自动降到 4:2:0，而 UI 截图存成 JPEG 时，4:2:0 会把彩色文字的  
边缘糊掉一大片——这类损失 `err` 的 99.9 分位未必拦得住（色度误差被亮度稀释）。

mozjpeg-rs 提供了 `set_chroma_sampling_pixel_sizes`。读原图 SOF 的抽样因子，4:4:4 的输入  
就保持 4:4:4。

### 26. 元数据只保了 ICC，其余全丢

- PNG：只贴回 `iCCP`。`pHYs`（DPI）、`eXIf`（拍摄参数/方向）、`tEXt/iTXt`（版权、作者）、  
  `gAMA` 全部丢失。**DPI 丢了，打印尺寸会变**；版权信息丢了是法律风险。
- JPEG：只保 ICC，EXIF / XMP / IPTC 全丢。方向已经烘进像素（`decode_jpeg_oriented` 做对了），  
  但拍摄时间和相机信息没了。

至少该保 `pHYs` 和 `eXIf`——`icc.rs` 已经有现成的 chunk 搬运逻辑，扩成白名单列表成本很低。

### 27. 感知分不认 ICC，一律按 sRGB/BT709 算

`quality.rs:111-117`：

```rust
Rgb::new(data, w, h, TransferCharacteristic::SRGB, ColorPrimaries::BT709)
```

而 `squash.rs` 又会把原图的 Display P3 profile 贴回输出。也就是说：**用 sRGB 的假设去判定  
一张 P3 图的可接受程度**。P3 的色域更大，同样的量化误差在 P3 下视觉上更小，闸门会偏保守  
（或对某些色相偏激进）。宽色域截图在 macOS 上很常见。

短期：在文档里写清这是 sRGB 假设，并把 P3 图的 `min_score` 单独放宽。  
长期：解析 iCCP 的 primaries，映射到 `ColorPrimaries::BT2020`/自定义。

### 28. alpha 簇反预乘的近似

`quantize.rs:199-222` 用「簇内平均 alpha」反预乘簇质心。数学上是对的（质心 = alpha 加权  
颜色均值），但一个簇的 alpha 跨度很大时（比如 90→255 的边缘簇），用单一 alpha 还原会让  
整簇颜色偏移。可以考虑按 alpha 分桶（0/64/128/192/255）后再量化，或者把半透明像素单独  
处理。当前对图标够用，属于可接受的取舍，但值得写进注释。

---

## 测试

- **格式转换零覆盖**：`grep outputFormat` 在所有测试里只有 `"keep"`。第 1、2、3、4、6 条  
  全在这条支路上，一条测试就能全逮住。
- **陈旧断言**：`squash.rs:920` 注释写「默认 min_score 现在是 0.98」（SSIM 时代），  
  断言却是 `score >= 70.0`——现在阈值是 78，这条断言已经不测任何东西了。
- **魔法数字**：`lib.rs:490` 的 `max_err: 24.0` 是 policy 改成 28 之前的旧值。  
  测试应该 `use tinypng_core::policy::*`，而不是抄数字。
- **死分支被测试养着**：`Labels.test.ts` 测 `pngquant-q` / `imagequant-q` 这些早就删掉的  
  引擎产出的 method 字符串，`queue.ts:65` 里对应的分支永远不会在真实数据上命中。

---

## 建议的动手顺序

1. 把 `output_format` 改成枚举，顺带修掉硬编码 `webp`、`max_err_convert` 不可配（P1 11/12）
2. 转换分支前移，不再白跑量化（P2 18）
3. 扩展名跟随格式（P0 1）——这个用户直接能看见
4. `kept_icc` 的三处修正 + `unchanged()` 带上真实 ICC 状态（P0 3）
5. hint 逻辑重写：删掉 `>200KB` 的粗暴判据，转换失败时把 `_e` 传给用户（P0 4/5）
6. 补 4 条转换路径的测试，覆盖 webp/avif × 有 ICC/无 ICC
7. 再谈 WebP 有损 / AVIF 质量搜索 / 色度抽样（P3 23/24/25）——这几个要换依赖，单独排期

---

# 处理结果（2026-10-27）

上面的 1–28 条已全部处理，`cargo clippy --all-targets` 两个 crate 零告警，
`nsh test` 69 项（45 Rust + 24 TS）全绿。

## 已修复

| 编号 | 处理方式 |
| --- | --- |
| 1 扩展名 | `Outcome.extension` 由 Rust 决定，`compress_file` 据此改写路径并回报 `output_path`；转换失败退回原格式时会把扩展名改回来，撞名自动加 `-2` 后缀 |
| 2 method 硬编码 | 转换路径的方法名直接是格式名（`webp`），不再拼后缀 |
| 3 kept_icc 说谎 | 删掉标志位参数，改成 `carries_icc(bytes)` 从输出字节流判定（PNG `iCCP` / JPEG APP2 / WebP `ICCP`） |
| 4 hint 文不对题 | `Conversion::Rejected(String)` 带上真实原因（误差多少、阈值多少、收益不足多少），直接展示 |
| 5 >200KB 就提示 | 整段删除，不再按字节数猜内容 |
| 6 无损/JPEG 忽略格式 | 转换是智能模式专属；无损模式下 UI 禁用选项，Rust 侧返回明确 hint。JPEG 也走同一条转换路径 |
| 7 ratio 两种舍入 | 统一成 `squash::ratio()`，四舍五入并夹到 0–100 |
| 8 input==output | 改比 `canonicalize()`，挡住符号链接与大小写 |
| 9 阈值两份 | `policy::Preset` 是唯一来源，`AutoArgs` 只传档位名；`Compress.presets()` 把数字给界面显示 |
| 10 结构体重复 | `FileOutcome { input_size, output_size, output_path, outcome }`，FFI 层从它映射 |
| 11 字符串枚举 | `OutputFormat` 枚举 + `parse`，解析失败在入口就报错 |
| 12 max_err_convert 不可配 | 并入 `Gate`，随档位走（严格档 18 / 标准 30 / 更小 38） |
| 13 死 API | 删除 `Compress.png` / `CompressArgs` / `quality` |
| 14 ZENQUANT_VERSION | 删除 |
| 15 Fix N 注释 | 全部改写成设计说明 |
| 16 to_file_url 死代码 | `FileRow` 的 `<Image src>` 改走 `Paths.toFileUrl()` |
| 17 pick_files 模态 | 挪到 `ambient::blocking`，模态循环不再占住异步执行器 |
| 18 转换前白跑量化 | 转换分支前移到量化之前，直接从解码后的原图编码 |
| 19 visible_error 分配 | 黑底/白底/alpha 三个直方图一次遍历累加，去掉 4 份缓冲和 4 次 clone |
| 20 score_over 分配 | 不透明图走直通分支，不再合成 |
| 21 best_candidate 比的是 oxipng 前体积 | 保留近似，但在文档里写明 |
| 22 依赖重复 | core 与 app 的 `png` 都升到 0.18；去掉 `image/avif`，`ravif`/`rav1e` 整个消失（Cargo.lock 少 274 行） |
| 23 WebP 是无损 | 文案改成「WebP 用无损编码，通常比 PNG 小 20–30%」，`lossy` 如实报 false |
| 25 色度抽样 | 从 SOF 读原图抽样因子并用 `set_chroma_sampling_pixel_sizes` 继承，有测试 |
| 26 元数据 | `png_meta::PRESERVED` 保留 `iCCP` / `pHYs` / `eXIf`；WebP 侧补 `ICCP` chunk |
| 27 ICC/P3 假设 | 在 `quality` 模块文档里写明这是 sRGB 假设，README 的「已知边界」也加了 |
| 测试 | 转换路径从 0 条补到 5 条；删掉陈旧的 `0.98` 注释和 `max_err: 24.0` 魔法数字；死分支不再被测试养着 |

## 改动过程中新发现的两个问题

1. **AVIF 从来没有工作过。** `image` 的 `avif` feature 只带编码器（ravif），解码要
   `avif-native`（dav1d）。原代码 `convert_format_from_bytes` 里
   `load_from_memory(&buf)?` 对 AVIF 必然失败，错误又被 `Err(_e)` 吞掉，用户最终拿到
   的是改名前的 PNG。因为无法回读就没法验闸门，**AVIF 已整体移除**（UI + Rust + 依赖），
   要恢复得先补解码器。
2. **给 WebP 补 ICCP 时会吃掉 alpha。** `image` 的无损编码只写 `VP8L`（alpha 在码流里），
   为了塞 `ICCP` 补 `VP8X` 时如果不置 `ALPHA_FLAG`，解码器按「不透明」处理，半透明
   像素整片变实心。已修，并有 `webp_carries_the_icc_profile_and_stays_lossless`
   这条测试守住（断言逐点一致）。

## 未做

- **有损 WebP**：已接入 `zenwebp` 0.4（纯 Rust，AGPL-3.0，和 zenquant 同一家）。
  有损质量二分 + 无损对照取小者，ICC 由库直接写 `ICCP`，手写的 `webp.rs` 已删除。
  实测 `for_test.png`（1.21 MB）：保持 PNG 83.2%，转 WebP 12.2%（q88，感知 79.8）。
- **oxipng 7 → 9**：`png 0.17` 还从 `oxipng → image 0.24` 那里来，升 oxipng 才能彻底
  去掉重复的 `png`/`image`。属于依赖升级，没网络环境验证，留给下一次。
- **P3 色域的精确判定**：需要解析 iCCP 里的 primaries，目前只在文档里说明假设。

---

# 处理结果（2026-11-03）：流程与策略核对轮

> 2026-10-27 改造后的全量复核（入口分流、搜索、验收闸门、大图、落盘、文档承诺逐条对照）。
> 结论：流程与策略全部属实，唯一阻塞缺陷是 oxipng 灰度降级使 iCCP 失配，已修复。

## 缺陷：oxipng 灰度降级使 iCCP 失配（已修复）

全灰像素（r=g=b）的 RGB/RGBA PNG 带 iCCP 时（扫描件、黑白图表导出常见），oxipng 把
颜色类型 2→0 / 6→4 降成灰度，但 RGB 色彩空间的 iCCP 原样保留。PNG 规范要求 iCCP 与
颜色类型匹配（类型 0/4 必须配 GRAY profile），输出因此不合规，libpng 系解码器会忽略
该配置——与「同格式重编码保留色彩解释」的承诺矛盾，且输出 `kept_icc: true`，全程静默。

- 修复：`png.rs optimize()` 对带 iCCP 的输入关闭 `grayscale_reduction`（oxipng 7.0.0
  公开字段，上游 CLI 亦有 `--no-grayscale-reduction`）。其余降级不受影响：2→3 调色板、
  6→2 仍属 RGB 系，配 RGB profile 合规；量化候选与流式路径不经过此处。
- 回归：`gray_pixels_with_iccp_are_not_downgraded_to_grayscale`。修复前实测 2→0（红），
  修复后输出为 RGB 系（绿）。断言是「颜色类型不得落入 0/4」而非逐字节不变——
  降调色板（2→3）无损且合规，不应误伤。
- 验证：core 41 通过；mac 63 通过、5 个基准跳过；两 crate `clippy -D warnings` 零告警；
  `cargo fmt` 无差异；release 实图验收（quality-check.sh）通过；改动仅 `png.rs` +50/−1。

## 次要观察（不阻塞发布）

- 灰度 JPEG（1 分量）若被重编码会变 3 分量 YCbCr（代码阅读结论；实测灰度样本被判
  无收益保留原样，未触发）。
- `Profile.grayscale` 只打日志，不参与决策。
- README 写 TS 测试 27 项，`nsh test` 实际约 32 项，数字略旧。
- Windows 只有 CI 配置，本机无法验证。

