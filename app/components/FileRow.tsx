import { Image, Text, View } from "nsh";
import { Button } from "nsh/base";
import { Paths } from "tinypng-mac/native";
import { colors as c, Icon, Pill } from "../ui";
import { fileDetail, fileDetailCompact, formatSize, ratioLabel, removeFile, revealOutput } from "../state/queue";
import type { FileItem } from "../state/queue";
import { layout } from "../state/layout";
import { recompress } from "../state/compression";

export function renderFileRow(props: { file: FileItem; busy: boolean; preview: boolean }) {
  const f = props.file;
  const l = layout();
  const compact = l.compact;
  const thumb = compact ? 36 : 46;
  return <View style={{ flexDirection: "row", alignItems: "center", minHeight: compact ? 62 : 78, paddingVertical: compact ? 9 : 13, gap: compact ? 10 : 12, borderBottomWidth: 1, borderColor: "#EEF0EC" }}>
    <View style={{ width: thumb, height: thumb, backgroundColor: "#EDF0EA", borderRadius: 8, overflow: "hidden", alignItems: "center", justifyContent: "center" }}>
      {/* `<Image src>` 只认 URL / asset:，Windows 的 `C:\` 会被当成 scheme，
          所以缩略图统一走 file:// 转换，别直接塞本地路径。 */}
      {f.preview && props.preview
        ? <Image src={Paths.toFileUrl(f.path)} resizeMode="contain" style={{ width: thumb, height: thumb }} accessibilityLabel={f.name} />
        : <Icon name="image" size={22} color="#93A88A" />}
    </View>
    <View style={{ flex: 1, minWidth: 0, gap: compact ? 4 : 6 }}>
      <Text numberOfLines={1} style={{ fontSize: compact ? 11.5 : 12, fontWeight: "500", color: c.ink }}>{f.name}</Text>
      <Text numberOfLines={f.status === "error" ? 2 : 1} style={{ fontSize: 10, lineHeight: 15, color: f.status === "error" ? "#BC6453" : "#98A093" }}>{compact ? fileDetailCompact(f) : fileDetail(f)}</Text>
      {f.status === "done" && f.hint && !compact && <Text numberOfLines={2} style={{ fontSize: 9, lineHeight: 13, color: "#8B7B4A" }}>{`💡 ${f.hint}`}</Text>}
    </View>
    {!compact && <View style={{ alignItems: "flex-end", gap: 6, minWidth: 80 }}>
      <Text style={{ fontSize: 11, color: c.muted }}>{formatSize(f.inputSize)}</Text>
      {f.status === "done" && <Text style={{ fontSize: 12, color: c.ink, fontWeight: "600" }}>{`→ ${formatSize(f.outputSize)}`}</Text>}
    </View>}
    <View style={{ alignItems: "center", width: compact ? 54 : 66, gap: 5 }}>
      {f.status === "done" ? <Pill text={ratioLabel(f)} green={f.ratio > 0} /> :
        <Text numberOfLines={2} style={{ fontSize: 10, textAlign: "center", color: f.status === "error" ? "#BC6453" : f.status === "compressing" ? c.green : "#97A08F" }}>{f.status === "compressing" ? "压缩中…" : f.status === "error" ? "待重试" : compact ? "等待" : "等待压缩"}</Text>}
    </View>
    <View style={{ flexDirection: "row", alignItems: "center", gap: 2 }}>
      {f.status === "done" && <Button onPress={() => void recompress(f.path)} disabled={props.busy}
        accessibilityLabel={`重新压缩 ${f.name}`} tooltip="使用当前设置从原图重新压缩，保留已有输出"
        style={{ padding: 5, borderRadius: 5, cursor: "pointer" }} hoverStyle={{ backgroundColor: "#F0F3ED" }} disabledStyle={{ opacity: 0.25 }}>
        <Icon name="refresh" size={13} color="#8B9A82" />
      </Button>}
      {f.status === "done" && f.outputPath && <Button onPress={() => void revealOutput(f)} accessibilityLabel={`在 Finder 中显示 ${f.name}`} tooltip="在 Finder 中显示"
        style={{ padding: 5, borderRadius: 5, cursor: "pointer" }} hoverStyle={{ backgroundColor: "#F0F3ED" }}><Icon name="folder" size={13} color="#8B9A82" /></Button>}
      <Button onPress={() => removeFile(f.path)} disabled={props.busy} accessibilityLabel={`移除 ${f.name}`} tooltip="从列表移除"
        style={{ padding: 5, borderRadius: 5, cursor: "pointer" }} hoverStyle={{ backgroundColor: "#F0F3ED" }} disabledStyle={{ opacity: 0.25 }}><Icon name="close" size={13} color="#A4ACA0" /></Button>
    </View>
  </View>;
}
