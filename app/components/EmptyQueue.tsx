import { Text, View } from "nsh";
import { Icon } from "../ui";
import { files, filter } from "../state/queue";

export function renderEmptyQueue() {
  return <View style={{ flex: 1, minHeight: 120, alignItems: "center", justifyContent: "center", gap: 10 }}>
    <Icon name={filter() === "done" ? "circleCheck" : "image"} size={28} color="#C2CBBA" />
    <Text style={{ fontSize: 12, color: "#99A290" }}>{filter() === "done" ? "完成的图片会显示在这里" : filter() === "pending" && files().length ? "全部处理完成，轻装上阵。" : "还没有图片，试着添加第一张吧"}</Text>
    <Text style={{ fontSize: 10, color: "#B2BAAA" }}>{files().length ? "切换左侧分类，查看其他图片" : "支持批量处理 · 原图始终保留"}</Text>
  </View>;
}
