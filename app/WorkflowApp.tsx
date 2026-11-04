import { mount, signal, Text, View } from "nsh";
import { Button } from "nsh/base";
import fs from "fs/promises";
import { Paths } from "tinypng-mac/native";
import { addFiles, busy, compressing, files, filter } from "./state/queue";
import { recompress, startCompress, stopCompress, stopped } from "./state/compression";
import { mode, preset, outputFormat } from "./state/settings";

function assert(condition: unknown, message: string) { if (!condition) throw new Error(message); }
function equal(a: Uint8Array, b: Uint8Array) { return a.length === b.length && a.every((byte, i) => byte === b[i]); }
async function cleanup(path: string): Promise<void> {
  for (const entry of await fs.readdir(path, { withFileTypes: true })) {
    const child = Paths.join(path, entry.name);
    if (entry.isDirectory()) await cleanup(child);
    else await fs.unlink(child);
  }
  await fs.rmdir(path);
}
const result = signal("等待验收");
async function run() {
  const directory = Paths.absolute(`tests/runtime-${Date.now()}`);
  await fs.mkdir(Paths.join(directory, "compressed"), { recursive: true });
  const input = Paths.join(directory, "icon.png");
  const original = await fs.readFile(Paths.absolute("tests/fixtures/icon-icc.png"));
  await fs.writeFile(input, original);
  await fs.writeFile(Paths.join(directory, "compressed/previous.png"), original);
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
    const firstBytes = await fs.readFile(first.outputPath!);
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
    assert(equal(await fs.readFile(input), original), "原图被修改");
    assert(equal(await fs.readFile(first.outputPath!), firstBytes), "原输出被修改");
    assert((await fs.readdir(Paths.join(directory, "compressed"))).length === 3, "重复启动或输出丢失");
    await fs.unlink(second.outputPath!);
    await recompress(second.path);
    assert(files()[0].status === "done", "输出删除后重压没有执行");
    assert(filter() === "all", "导入后未回到全部列表");
    const extra = [];
    for (let i = 0; i < 12; i++) {
      const path = Paths.join(directory, `extra-${i}.png`);
      await fs.writeFile(path, original);
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
    for (const file of remaining) assert(await fs.exists(file.outputPath!), "未启动项的旧输出被删除");
    await startCompress();
    assert(files().every(file => file.status === "done"), "继续后仍有遗漏任务");
  } finally {
    files.set([]);
    await cleanup(directory);
  }
}
export default function WorkflowApp() {
  return <View><Button onPress={() => { void run().then(() => result.set("验收通过"), error => result.set(`验收失败：${String(error)}`)); }}><Text>运行验收</Text></Button><Text>{result()}</Text></View>;
}
mount(WorkflowApp);
