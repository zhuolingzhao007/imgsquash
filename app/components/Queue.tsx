import { Scroll, Text, View } from "nsh";
import { Button } from "nsh/base";
import { colors as c, Action, Icon, Pill } from "../ui";
import { busy, checkFiles, clearCompleted, filter, pickFiles, pickFolders, totals, visibleFiles } from "../state/queue";
import { layout } from "../state/layout";
import { renderEmptyQueue } from "./EmptyQueue";
import { renderFileRow } from "./FileRow";

export function renderQueue() {
  const l = layout();
  const title = filter() === "done" ? "已完成" : filter() === "pending" ? "待处理" : "图片列表";
  return <View style={{ flex: 1, minHeight: 0 }}>
    <View style={{ flexDirection: "row", alignItems: "center", marginTop: l.short ? 14 : 22, marginBottom: l.compact ? 8 : 12, gap: 6 }}>
      <Text numberOfLines={1} style={{ fontSize: 13, fontWeight: "600", color: c.ink }}>{title}</Text>
      <Pill text={`${visibleFiles().length}`} />
      <View style={{ flex: 1 }} />
      <Button onPress={() => void checkFiles()} disabled={busy()} accessibilityLabel="检查文件状态" tooltip="和磁盘对一次账：删掉的文件会移除，输出没了的会退回待处理"
        style={{ padding: 6, borderRadius: 6, cursor: "pointer" }} hoverStyle={{ backgroundColor: "#F0F3ED" }} disabledStyle={{ opacity: 0.3 }}>
        <Icon name="refresh" size={13} color={c.muted} />
      </Button>
      {totals().done > 0 && <Button onPress={clearCompleted} disabled={busy()} style={{ padding: 6, borderRadius: 6, cursor: "pointer" }} hoverStyle={{ backgroundColor: "#F0F3ED" }} disabledStyle={{ opacity: 0.4 }}>
        <Text numberOfLines={1} style={{ fontSize: 11, color: c.muted }}>清除已完成</Text>
      </Button>}
      <Action label="添加图片" icon="plus" small onPress={pickFiles} disabled={busy()} accessibilityLabel="添加图片" tooltip="添加图片（⌘O）" />
      <Action label="添加文件夹" icon="folder" small iconOnly={l.compact} onPress={pickFolders} disabled={busy()} accessibilityLabel="添加文件夹" tooltip="扫描文件夹内的 PNG / JPEG" />
    </View>
    {!l.compact && <View style={{ flexDirection: "row", alignItems: "center", paddingBottom: 12, borderBottomWidth: 1, borderColor: c.line }}>
      <Text style={{ flex: 1, fontSize: 10, color: "#A0A896" }}>文件名称</Text>
      <Text style={{ fontSize: 10, color: "#A0A896", width: 90, textAlign: "right", marginRight: 20 }}>文件大小</Text>
      <Text style={{ fontSize: 10, color: "#A0A896", width: 66, textAlign: "center" }}>状态</Text>
    </View>}
    {visibleFiles().length
      ? <Scroll style={{ flex: 1, minHeight: 0 }}>{visibleFiles().map((file, index) => renderFileRow({ file, busy: busy(), preview: index < 32 }))}</Scroll>
      : renderEmptyQueue()}
  </View>;
}
