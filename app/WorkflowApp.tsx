import { mount, signal, Text, View } from "nsh";
import { Button } from "nsh/base";
import { homedir } from "os";
import { Files, Paths } from "tinypng-mac/native";
import { addFiles, busy, compressing, files, filter } from "./state/queue";
import { recompress, startCompress, stopCompress, stopped } from "./state/compression";
import { mode, preset, outputFormat } from "./state/settings";

function assert(condition: unknown, message: string) { if (!condition) throw new Error(message); }
function equal(a: Uint8Array, b: Uint8Array) { return a.length === b.length && a.every((byte, i) => byte === b[i]); }
async function cleanup(path: string): Promise<void> {
  for (const entry of await Files.readDir(path)) {
    const child = Paths.join(path, entry.name);
    if (entry.isDirectory) await cleanup(child);
    else await Files.unlink(child);
  }
  await Files.rmdir(path);
}

// 带 iCCP 的样张。内嵌而非读 tests/fixtures：nsh 的 capabilities.fs 按
// ${homeDir} 等授权根做前缀匹配，CI 的 checkout 在 D 盘，仓库内路径天然越界；
// Buffer 是 nsh 的全局对象（LLRT），解码不需要任何额外授权。
const ICON_ICC_PNG = "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";

const result = signal("等待验收");
async function run() {
  // 运行区放在家目录：capabilities.fs 只按 ${homeDir} 等授权根做前缀匹配，
  // CI 的 checkout 在 D 盘，仓库内路径天然越界（os.homedir() 无需授权）。
  const root = Paths.join(homedir(), ".imgsquash-test");
  const directory = Paths.join(root, `runtime-${Date.now()}`);
  await Files.mkdir(Paths.join(directory, "compressed"), true);
  const input = Paths.join(directory, "icon.png");
  const original = Buffer.from(ICON_ICC_PNG, "base64");
  await Files.writeFile(input, original);
  await Files.writeFile(Paths.join(directory, "compressed/previous.png"), original);
  files.set([]);
  mode.set("smart"); preset.set("standard"); outputFormat.set("keep");
  try {
    await addFiles([directory]);
    assert(files().length === 1, "输出目录不应被重复导入");
    assert(files()[0].preview, "小图应启用预览");
    await Promise.all([startCompress(), startCompress()]);
    assert(!busy(), "批次未释放状态");
    const first = files()[0];
    assert(first.status === "done", first.error || "首次压缩失败");
    const firstBytes = await Files.readFile(first.outputPath!);
    assert(first.method === "quantizr-c3", "未使用智能模式");
    mode.set("lossless");
    // 即使上次记住 WebP，无损模式的有效输出也必须是原格式。
    outputFormat.set("webp");
    await recompress(first.path);
    const second = files()[0];
    assert(second.status === "done", second.error || "重压失败");
    assert(second.method === "oxipng", "未使用当前无损设置");
    assert(second.outputPath !== first.outputPath, "重压覆盖了上次输出");
    assert(second.outputPath!.endsWith(".png"), "无损模式不应转换格式");
    assert(equal(await Files.readFile(input), original), "原图被修改");
    assert(equal(await Files.readFile(first.outputPath!), firstBytes), "原输出被修改");
    assert((await Files.readDir(Paths.join(directory, "compressed"))).length === 3, "重复启动或输出丢失");
    await Files.unlink(second.outputPath!);
    await recompress(second.path);
    assert(files()[0].status === "done", "输出删除后重压没有执行");
    assert(filter() === "all", "导入后未回到全部列表");
    const extra = [];
    for (let i = 0; i < 12; i++) {
      const path = Paths.join(directory, `extra-${i}.png`);
      await Files.writeFile(path, original);
      extra.push(path);
    }
    await addFiles(extra);
    await startCompress();
    assert(files().every(file => file.status === "done"), "首次批处理未完成");
    const repeated = recompress();
    for (let i = 0; !compressing() && i < 1000; i++) await new Promise(resolve => setTimeout(resolve, 1));
    assert(compressing(), "重压批次没有启动");
    stopCompress();
    await repeated;
    assert(stopped(), "重压没有停止");
    const remaining = files().filter(file => file.status === "idle");
    assert(remaining.length > 0, "未启动的重压任务应保持待处理");
    for (const file of remaining) assert(await Files.exists(file.outputPath!), "未启动项的旧输出被删除");
    await startCompress();
    assert(files().every(file => file.status === "done"), "继续后仍有遗漏任务");
  } finally {
    files.set([]);
    await cleanup(root);
  }
}
export default function WorkflowApp() {
  return <View><Button onPress={() => { void run().then(() => result.set("验收通过"), error => result.set(`验收失败：${String(error)}`)); }}><Text>运行验收</Text></Button><Text>{result()}</Text></View>;
}
mount(WorkflowApp);
