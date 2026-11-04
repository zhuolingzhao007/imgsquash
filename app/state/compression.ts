import { signal } from "nsh";
import { Compress, Files, Paths } from "tinypng-mac/native";
import { addSpent, busy, compressing, files, notice, preparing, syncMessage, syncWithDisk, updateFile } from "./queue";
import type { FileItem } from "./queue";
import { effectiveOutputFormat, mode, preset } from "./settings";
import { runPool } from "./pool";
import { candidateName, reservationKey } from "./naming";

export const progress = signal(0);
/** 已经压完的张数。 */
export const finished = signal(0);
/** 正在压的张数，并行时大于 1。 */
export const active = signal(0);
export const queueTotal = signal(0);
export const elapsed = signal(0);
export const stopping = signal(false);
export const stopped = signal(false);

/** 不再领取任务；已经交给 Rust 的任务安全完成后收尾。 */
export function stopCompress() {
  if (compressing()) stopping.set(true);
}

/** 从原图重压，保留之前输出；不把上一次的有损输出作为新输入。 */
export async function recompress(path?: string) {
  const paths = files().filter(file => file.status === "done" && (!path || file.path === path)).map(file => file.path);
  if (paths.length) await startCompress(paths);
}

interface RunOptions {
  mode: string;
  /** 档位名，画质偏好由 Rust 核心策略统一解释。 */
  preset: string;
  outputFormat: string;
}

/**
 * 输出文件名：选了目标格式就把扩展名换掉。
 *
 * 以前一直沿用原扩展名，于是 `pic.png` 里装着 WebP 字节——Finder 打不开，
 * 上传到认后缀的服务直接报错。扩展名用 `Paths.withExtension` 换，别在 TS 里
 * 用 `split(".")`，Windows 的路径和带点的文件名都会踩坑。
 */
function outputName(name: string, format: string) {
  return format === "keep" ? name : Paths.withExtension(name, format);
}

/**
 * 给这张图占一个输出路径。两件事：
 * - 同一批是并行跑的，`Files.exists` 只反映磁盘，反映不了「另一个任务马上要写这个名字」，
 *   所以先用内存里的 `reserved` 占位。
 * - 键统一转小写：APFS 默认不分大小写，pic.png 和 Pic.png 是同一个文件。
 */
async function reserveOutput(file: FileItem, reserved: Set<string>, format: string) {
  const directory = Paths.join(Paths.parent(file.path), "compressed");
  await Files.mkdir(directory, true);
  const wanted = outputName(file.name, format);

  for (let suffix = 1; ; suffix++) {
    const candidate = Paths.join(directory, candidateName(wanted, suffix));
    const key = reservationKey(candidate);
    if (!reserved.has(key) && !(await Files.exists(candidate))) {
      // await 之后重新确认再同步占位：中间没有 await，别的任务插不进来。
      if (!reserved.has(key)) {
        reserved.add(key);
        return candidate;
      }
    }
  }
}

async function compressOne(file: FileItem, reserved: Set<string>, options: RunOptions) {
  updateFile(file.path, { status: "compressing", error: undefined });
  try {
    const output = await reserveOutput(file, reserved, options.outputFormat);
    const result = await Compress.auto({
      input: file.path,
      output,
      mode: options.mode,
      preset: options.preset,
      outputFormat: options.outputFormat,
    });
    updateFile(file.path, {
      inputSize: result.inputSize,
      outputSize: result.outputSize,
      // 以 Rust 回报的路径为准：转换收益不足时它会退回原格式，扩展名也跟着变。
      outputPath: result.outputPath,
      ratio: result.ratio,
      method: result.method,
      score: result.score,
      scoreKind: result.scoreKind,
      lossy: result.lossy,
      keptIcc: result.keptIcc,
      hint: result.hint ?? undefined,
      status: "done",
    });
  } catch (error) {
    updateFile(file.path, { status: "error", error: String(error) });
  }
}

/**
 * 压缩队列。多张并行：`Compress.auto` 走的是 tokio 的阻塞线程池，
 * Rust 文件边界协调内存准入，TS 只限制同时提交数量并处理停止。
 */
export async function startCompress(repeatPaths?: readonly string[]) {
  if (busy()) return;
  preparing.set(true);
  try {
    // 先把列表和磁盘对一次账：原图被删掉的不能进队列，输出被删掉的要重压。
    const synced = await syncWithDisk();
    const syncedNotice = syncMessage(synced);
    const repeats = repeatPaths ? new Set(repeatPaths) : undefined;
    const queue = files().filter(f => repeats
      ? repeats.has(f.path)
      : f.status === "idle" || f.status === "error");
    if (!queue.length) {
      if (syncedNotice) notice.set(syncedNotice);
      return;
    }

    const options: RunOptions = {
      mode: mode(),
      preset: preset(),
      outputFormat: effectiveOutputFormat(),
    };
    const workers = Math.min(Compress.parallelism(), queue.length);

    // 重压也是一批待处理任务；这样停止后能只继续剩余项，而非重压所有完成项。
    // 只改变任务状态，保留上次输出路径和磁盘文件。
    if (repeats) {
      for (const file of queue) updateFile(file.path, { status: "idle", error: undefined });
    }
    compressing.set(true);
    preparing.set(false);
    stopping.set(false); stopped.set(false);
    progress.set(0); finished.set(0); active.set(0); queueTotal.set(queue.length);
    notice.set(syncedNotice);
    const startedAt = Date.now();
    const timer = setInterval(() => elapsed.set(Date.now() - startedAt), 200);
    const reserved = new Set<string>();
    let done = 0;

    try {
      await runPool(queue, workers, async file => {
        active.update(n => n + 1);
        try {
          await compressOne(file, reserved, options);
        } finally {
          active.update(n => n - 1);
          done += 1;
          finished.set(done);
          progress.set(Math.round((done / queue.length) * 100));
        }
      }, stopping);
      const failed = files().filter(f => f.status === "error").length;
      stopped.set(stopping() && done < queue.length);
      const messages = [];
      if (stopped()) messages.push(`已停止，${queue.length - done} 张未启动；已完成的输出保留。`);
      if (failed) messages.push(`${failed} 张图片处理失败，可再次重试。`);
      if (messages.length) notice.set(messages.join(" "));
    } finally {
      // 墙钟耗时按批次累加：底部总结里的「用时」和累计节省是同一口径，
      // 分几次压也不会对不上。elapsed 是压缩中的实时计时，收尾要归零。
      const runMs = Date.now() - startedAt;
      clearInterval(timer);
      compressing.set(false);
      stopping.set(false);
      active.set(0);
      finished.set(0);
      queueTotal.set(0);
      elapsed.set(0);
      if (done > 0) addSpent(runMs);
    }
  } finally { preparing.set(false); }
}
