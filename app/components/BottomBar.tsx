import { Text, View } from "nsh";
import { colors as c, Action } from "../ui";
import { busy, compressing, files, formatDuration, formatSize, openOutputFolder, spentMs, totals } from "../state/queue";
import { active, elapsed, finished, progress, queueTotal, recompress, startCompress, stopCompress, stopped, stopping } from "../state/compression";
import { layout } from "../state/layout";

function label() {
  if (compressing()) return "正在压缩…";
  if (files().length && files().every(f => f.status === "done")) return "按当前设置重压";
  if (stopped()) return "继续压缩";
  if (files().some(f => f.status === "error") && !files().some(f => f.status === "idle")) return "重试失败图片";
  return "开始压缩";
}

function lines() {
  const parallel = active() > 1 ? `${active()} 张并行 · ` : "";
  const t = totals();
  // 压缩中报实时计时，压完报累计墙钟耗时——两者都是「这一次等了多久」的口径。
  const spent = t.done && spentMs() ? ` · 用时 ${formatDuration(spentMs())}` : "";
  return {
    primary: compressing() ? `已完成 ${finished()}/${queueTotal()} 张` : `${files().length} 张图片 · ${formatSize(files().reduce((sum, f) => sum + f.inputSize, 0))}`,
    secondary: compressing()
      ? `${parallel}已用 ${formatDuration(elapsed())}`
      : t.done
        ? `已完成 ${t.done} 张 · 节省 ${formatSize(t.saved)}${spent}`
        : "准备好了，就让它们轻一点",
  };
}

export function renderBottomBar() {
  const l = layout();
  const text = lines();
  return <View style={{ borderTopWidth: 1, borderColor: c.line, paddingVertical: l.short ? 14 : 18, gap: 10 }}>
    {compressing() && <View style={{ height: 3, borderRadius: 2, overflow: "hidden", flexDirection: "row", backgroundColor: "#E9EEE3" }}>
      <View style={{ flexGrow: progress(), flexBasis: 0, backgroundColor: c.green }} />
      <View style={{ flexGrow: 100 - progress(), flexBasis: 0 }} />
    </View>}
    <View style={{ flexDirection: "row", alignItems: "center", gap: 10 }}>
      <View style={{ flex: 1, minWidth: 0, gap: 5 }}>
        <Text numberOfLines={1} style={{ fontSize: 11, color: "#75826C" }}>{text.primary}</Text>
        <Text numberOfLines={1} style={{ fontSize: 10, color: "#A4AD9A" }}>{text.secondary}</Text>
      </View>
      {compressing() && <Action testID="stop" label={stopping() ? "等待当前图片完成" : "停止队列"} small onPress={stopCompress}
        disabled={stopping()} accessibilityLabel="停止队列" tooltip="不再启动新图片；正在处理的图片会完成并安全保存" />}
      {totals().done > 0 && <Action label="打开文件夹" icon="folder" small onPress={openOutputFolder} disabled={busy()} accessibilityLabel="打开输出文件夹" tooltip="打开输出文件夹" />}
      <Action testID="compress" label={label()} icon="compress" primary onPress={() => {
        if (files().some(f => f.status === "idle" || f.status === "error")) void startCompress();
        else void recompress();
      }}
        disabled={busy() || !files().length}
        accessibilityLabel={compressing() ? "正在压缩" : label()} tooltip={files().length ? "使用当前设置处理原图（⌘↩）" : "先添加图片"} />
    </View>
  </View>;
}
