# 智能压缩接入 TinyPNG Local

## 目标

把 `imgsquash` 的「多档候选 + 质量闸门 + 收益判断」搬进 app，压缩时不再依赖 pngquant、oxipng、cjpeg 这些外部命令。

## 结构

- `../core`：CLI 与 app 共用的质量库，包含 SSIM、可见误差和默认阈值。两边判定必须一致。
- `src/squash.rs`：app 内后端。
  - PNG：imagequant 按 65 / 75 / 85 / 95 四档量化，每档用 `png` crate 写索引色，再跑 oxipng。第一档过闸门就采用，全不过退回 oxipng 无损。
  - JPEG：mozjpeg 在 60 到 95 之间二分，保留 ICC。
- `src/lib.rs`：`Compress.auto` 原生接口，CPU 工作放到 tokio 的 blocking 线程，界面不冻结。
- `state/settings.ts`：模式、档位和持久化。
- `state/queue.ts`：文件队列、文件夹扫描、统计、Finder。
- `state/compression.ts`：压缩流程、进度和当前文件。
- `state/shortcuts.ts`：⌘O / ⌘↩。
- `state/view.ts`：拖拽状态。
- `components/`：TopBar、PageHeader、Sidebar、Dropzone、Notice、Queue、FileRow、EmptyQueue、BottomBar、Settings、Stat。
- `App.tsx`：只做窗口、布局和组合。
- `compression.ts`：兼容入口，re-export `state/`。

NSH 只跟踪渲染作用域里的 signal 读取，视图层用返回 JSX 的 render 函数，由 `App` 内联调用。这样拆文件不会让选中状态或队列状态停在旧值。

## 判定

默认 SSIM ≥ 0.985，99.9 分位可见误差 ≤ 24，收益不足 3% 原样输出。

透明像素的 RGB 不参与误差统计，按黑底和白底合成后的颜色算。否则透明背景图标会被误杀。

PNG 遇到 APNG、带 ICC、16 位时退回无损。APNG 直接原样输出，避免丢帧。JPEG 遇到 CMYK 或 ICC 分片不完整时原样输出。

## 性能

release 构建，M 系列 Mac：

- 2360 × 1600 PNG 截图：约 4.4 秒，303 KB → 86 KB。
- 2360 × 1600 JPEG：约 1.5 秒，228 KB → 104 KB。

debug 构建里 imagequant 没优化，同一张 PNG 要七十多秒。开发和演示用 `nsh run --release`，打包产物默认 release。

## 体验

- 文件面板和拖拽都支持文件夹，递归收集图片，跳过 `compressed`、`node_modules`、`build` 等目录。
- 压缩中显示当前文件、队列位置和已用时间。
- 完成的文件行可以在 Finder 中显示，底部可以打开最后一张图的输出目录。
- 压缩方式和质量档位存进 `nsh/storage`，下次打开沿用。
- ⌘O 添加图片，⌘↩ 开始压缩。

## 验证

- Rust：有损 PNG 变小且 SSIM 达标、无损 PNG 像素一致、APNG 原样、JPEG 变小、`compress_file` 不改原图。
- UI：空状态与筛选、模式切换后质量档位显示/隐藏。
- 与 CLI 的 SSIM 共用同一份实现，避免两边对同一张图给出不同结论。
