import { signal } from "nsh";
import { Compress, FilePicker, Files, Finder, Paths } from "tinypng-mac/native";

export interface FileItem {
  path: string;
  name: string;
  format: "PNG" | "JPEG";
  inputSize: number;
  outputSize: number;
  outputPath?: string;
  ratio: number;
  status: "idle" | "compressing" | "done" | "error";
  error?: string;
  method: string;
  /** SSIMULACRA2 感知分，0–100，越高越好。 */
  score: number;
  scoreKind?: string;
  lossy: boolean;
  keptIcc: boolean;
  hint?: string;
  /** 原生 Image 可能解码整图，仅为有界的小图提供预览。 */
  preview?: boolean;
}

export const files = signal<FileItem[]>([]);
export const compressing = signal(false);
export const importing = signal(false);
export const syncing = signal(false);
export const preparing = signal(false);
const picking = signal(false);
export const notice = signal("");
export const filter = signal<"all" | "pending" | "done">("all");

const SKIP_DIRS = new Set(["node_modules", "Pods", ".git", "build", "DerivedData", "target", "compressed"]);

function isImageName(name: string) {
  return /\.(png|jpe?g)$/i.test(name);
}

/** 累计压缩耗时（毫秒，墙钟）。底部总结和统计卡都用它报「用时」。 */
export const spentMs = signal(0);

export function addSpent(ms: number) {
  if (ms > 0) spentMs.update(total => total + ms);
}

export function formatSize(bytes: number) {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

/**
 * 压缩耗时的读法：不到一秒给毫秒（小图看这个才有意义），
 * 一分钟以上换成 `1m23s`，避免出现 `83.4s` 这种要心算的写法。
 */
export function formatDuration(ms: number) {
  if (ms < 1000) return `${Math.round(ms)}ms`;
  if (ms < 60_000) return `${(ms / 1000).toFixed(1)}s`;
  const minutes = Math.floor(ms / 60_000);
  const seconds = Math.round((ms % 60_000) / 1000);
  return seconds ? `${minutes}m${seconds}s` : `${minutes}m`;
}

export function formatOf(name: string): "PNG" | "JPEG" {
  return /\.jpe?g$/i.test(name) ? "JPEG" : "PNG";
}

/**
 * `method` 是 Rust 侧给的短标识，这里翻成界面文案。
 *
 * 转换过的图方法名是 `webp`（无损胜出）或 `webp-qNN`（有损质量）。
 */
export function methodLabel(method: string) {
  if (method === "stream-png-lossless") return "无损 · 低内存";
  if (method.startsWith("stream-png-c")) return `${method.slice("stream-png-c".length)} 色 · 低内存`;
  if (method.startsWith("stream-jpeg-q")) return `q${method.slice("stream-jpeg-q".length)} · 低内存`;
  if (method.startsWith("quantizr-c")) return `${method.slice("quantizr-c".length)} 色`;
  if (method.startsWith("mozjpeg-q")) return `q${method.slice("mozjpeg-q".length)}`;
  if (method.startsWith("webp-q")) return `WebP q${method.slice("webp-q".length)}`;
  // 无损 WebP 在率失真比较中胜出。
  if (method === "webp") return "WebP 无损";
  if (method === "oxipng-16") return "无损 · 16 位深";
  if (method === "oxipng") return "无损";
  if (method === "original") return "原样";
  return "已最优";
}

export function fileDetail(file: FileItem) {
  if (file.status === "error") return file.error || "处理失败";
  if (file.status === "done") {
    const method = methodLabel(file.method);
    const quality = file.lossy ? ` · ${file.scoreKind === "tiled" ? "分块最低" : "感知"} ${file.score.toFixed(1)}` : "";
    const icc = file.keptIcc ? " · 保留 ICC" : "";
    return `${file.format} · ${method}${quality}${icc}`;
  }
  return `${file.format} · 保持原始尺寸`;
}

/** 窄列表用的一行摘要：体积变化放在最前，被截断时先丢的是方法细节。 */
export function fileDetailCompact(file: FileItem) {
  if (file.status === "error") return file.error || "处理失败";
  if (file.status === "done") {
    const quality = file.lossy ? ` · ${file.scoreKind === "tiled" ? "分块最低" : "感知"} ${file.score.toFixed(1)}` : "";
    return `${formatSize(file.inputSize)} → ${formatSize(file.outputSize)} · ${methodLabel(file.method)}${quality}`;
  }
  return `${formatSize(file.inputSize)} · ${file.format} · 保持原始尺寸`;
}

export function ratioLabel(file: FileItem) {
  if (file.ratio > 0) return `−${file.ratio}%`;
  if (file.method === "original") return "原样";
  if (file.method === "kept") return "无收益";
  return "已优化";
}

export function busy() {
  return compressing() || importing() || syncing() || picking() || preparing();
}

export function visibleFiles() {
  return files().filter(f => filter() === "all" || (filter() === "done" ? f.status === "done" : f.status !== "done"));
}

export function totals() {
  const done = files().filter(f => f.status === "done");
  const before = done.reduce((sum, f) => sum + f.inputSize, 0);
  const after = done.reduce((sum, f) => sum + f.outputSize, 0);
  return { done: done.length, before, after, saved: Math.max(0, before - after), ratio: before ? Math.round((1 - after / before) * 100) : 0 };
}

export async function addFiles(paths: string[]) {
  if (busy()) return;
  importing.set(true); notice.set("");
  const problems: string[] = [];
  try {
    const added: FileItem[] = [];
    const seen = new Set(files().map(f => f.path));

    // 拖入的 URI 不能直接 stat：macOS 可能给的是文件引用 URL（file:///.file/id=…，
    // 从别的应用拖图时常见），Windows 给的是 file:///C:/…。解析统一交给原生侧，
    // 那边按平台各用各的办法，解析不了会原样返回。
    let resolved = paths;
    try { resolved = FilePicker.resolveUris(paths); } catch { /* 解析不可用就按原值走 */ }

    const addOne = async (path: string, report: boolean) => {
      if (seen.has(path)) return;
      const name = Paths.base(path);
      if (!isImageName(name)) {
        if (report) problems.push(`${name}：目前支持 PNG / JPEG`);
        return;
      }
      const info = await Files.stat(path);
      if (!info.isFile) return;
      let preview = false;
      try {
        const dimensions = await Compress.inspect(path);
        preview = dimensions.width * dimensions.height <= 1024 * 1024;
      } catch { /* 保留损坏项，让压缩阶段报告错误；不尝试解码预览。 */ }
      seen.add(path);
      added.push({ path, name, format: formatOf(name), inputSize: info.size, outputSize: 0, ratio: 0, status: "idle", method: "", score: 100, lossy: false, keptIcc: false, preview });
    };

    const collect = async (path: string, depth: number): Promise<number> => {
      if (depth > 8) return 0;
      const info = await Files.stat(path);
      if (info.isFile) {
        if (!isImageName(path)) return 0;
        await addOne(path, false);
        return 1;
      }
      if (!info.isDirectory) return 0;
      let entries;
      try {
        entries = await Files.readDir(path);
      } catch (error) {
        problems.push(`无法读取文件夹 ${Paths.base(path)}：${String(error)}`);
        return 0;
      }
      let count = 0;
      for (const entry of entries) {
        if (entry.name.startsWith(".")) continue;
        if (entry.isDirectory && SKIP_DIRS.has(entry.name)) continue;
        const child = Paths.join(path, entry.name);
        try {
          count += await collect(child, depth + 1);
        } catch (error) {
          problems.push(`无法读取 ${entry.name}：${String(error)}`);
        }
      }
      return count;
    };

    for (const raw of resolved) {
      try {
        const path = Paths.absolute(raw);
        const info = await Files.stat(path);
        if (info.isDirectory) {
          const count = await collect(path, 0);
          if (count === 0) problems.push(`${Paths.base(path)}：没有找到 PNG / JPEG`);
        } else {
          await addOne(path, true);
        }
      } catch (error) {
        const label = Paths.base(raw);
        const detail = String(error);
        // /.file/id= 是 macOS 的文件引用 URL，别的平台不会出现。
        if (raw.includes("/.file/id=")) {
          problems.push(`${label}：这个文件来自其他应用，无法定位到磁盘文件，请先保存到本地或用「选择图片」导入`);
        } else if (detail.includes("not granted")) {
          problems.push(`${label}：没有读取权限，请把文件移到桌面或下载文件夹再试`);
        } else {
          problems.push(`无法读取 ${label}：${detail}`);
        }
      }
    }

    files.set([...files(), ...added]);
    if (added.length) { filter.set("all"); }
    if (problems.length) {
      const shown = problems.slice(0, 3).join("；");
      notice.set(problems.length > 3 ? `${shown}；另有 ${problems.length - 3} 条` : shown);
    }
  } finally { importing.set(false); }
}

export async function pickFiles() {
  await pickPaths(() => FilePicker.pickFiles());
}

export async function pickFolders() {
  await pickPaths(() => FilePicker.pickFolders());
}

async function pickPaths(select: () => Promise<string[]>) {
  if (busy()) return;
  picking.set(true);
  try {
    const paths = await select();
    picking.set(false);
    await addFiles(paths);
  }
  catch (error) { notice.set(`无法打开图片：${String(error)}`); }
  finally { picking.set(false); }
}

export function removeFile(path: string) {
  if (busy()) return;
  const rest = files().filter(f => f.path !== path);
  files.set(rest);
  // 列表清空了就把累计耗时一起归零，免得下一批的「用时」带着上一批的账。
  if (!rest.length) spentMs.set(0);
}

/** 按路径改一行。同步读改写，并发压缩时多个 worker 同时收尾也不会互相覆盖。 */
export function updateFile(path: string, patch: Partial<FileItem>) {
  files.set(files().map(f => (f.path === path ? { ...f, ...patch } : f)));
}

export function clearCompleted() {
  if (busy()) return;
  files.set(files().filter(f => f.status !== "done"));
}

export interface SyncResult {
  /** 原图已经不在磁盘上的文件名。 */
  removed: string[];
  /** 输出被删掉、退回待处理的文件名。 */
  reset: string[];
}

/**
 * 让列表回到磁盘的真实状态。列表只是一份快照，用户在 Finder 里删了原图
 * 或输出之后它不会自己知道：
 * - 原图没了：这张图不可能再压缩，直接从列表移除。
 * - 已完成但输出没了：收益和「在 Finder 中显示」都是假的，退回待处理。
 *
 * 读不到（权限、临时故障）一律当作还在：宁可留一行假的，也不要误删用户的队列。
 */
export async function syncWithDisk(): Promise<SyncResult> {
  const empty: SyncResult = { removed: [], reset: [] };
  if (syncing() || compressing()) return empty;
  syncing.set(true);
  try {
    const removed: string[] = [];
    const reset: string[] = [];
    const gone = new Set<string>();
    const stale = new Set<string>();
    for (const file of files()) {
      if (!(await stillThere(file.path))) { gone.add(file.path); removed.push(file.name); continue; }
      if (file.status === "done" && file.outputPath && !(await stillThere(file.outputPath))) {
        stale.add(file.path);
        reset.push(file.name);
      }
    }
    if (gone.size || stale.size) {
      // 扫盘要 await，期间可能有别的更新（比如并发压缩收尾）。这里一次性同步
      // 应用：按路径过滤/改写当前列表，而不是拿扫描开始时的快照整表覆盖。
      files.set(files()
        .filter(f => !gone.has(f.path))
        .map(f => (stale.has(f.path) && f.status === "done"
          ? { ...f, status: "idle", outputPath: undefined, outputSize: 0, ratio: 0, method: "", score: 100, lossy: false, hint: undefined }
          : f)));
    }
    return { removed, reset };
  } finally { syncing.set(false); }
}

async function stillThere(path: string) {
  try { return await Files.exists(path); } catch { return true; }
}

export function syncMessage(result: SyncResult) {
  const parts: string[] = [];
  if (result.removed.length) parts.push(`已移除 ${result.removed.length} 个不在磁盘上的文件`);
  if (result.reset.length) parts.push(`${result.reset.length} 张图的输出已不存在，已退回待处理`);
  return parts.join("；");
}

/** 队列头部的「检查状态」：和磁盘对一次账。 */
export async function checkFiles() {
  if (busy()) return;
  // 空队列对账什么也没扫，别回一句「文件都在」——一个文件都没有，谈不上都在。
  if (!files().length) {
    notice.set("队列是空的，先添加图片再检查。");
    return;
  }
  const result = await syncWithDisk();
  notice.set(syncMessage(result) || "文件都在，状态正常。");
}

export async function revealOutput(file: FileItem) {
  if (!file.outputPath) return;
  try { await Finder.reveal(file.outputPath); }
  catch (error) { notice.set(`无法在 Finder 中显示：${String(error)}`); }
}

export async function openOutputFolder() {
  const done = files().filter(f => f.status === "done" && f.outputPath);
  if (!done.length) return;
  const last = done[done.length - 1].outputPath!;
  const folder = Paths.parent(last) || last;
  try { await Finder.openFolder(folder); }
  catch (error) { notice.set(`无法打开输出文件夹：${String(error)}`); }
}
