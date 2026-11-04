import { Text, View } from "nsh";
import { Button } from "nsh/base";
import { colors as c, Icon } from "../ui";
import { pickFiles, pickFolders } from "../state/queue";
import { dragging } from "../state/view";
import { layout } from "../state/layout";

/**
 * 拖拽本身挂在根视图上，所以这里只负责画。有图之后收成一条窄条，
 * 把竖向空间留给列表。
 */
export function renderDropzone(props: { busy: boolean; hasFiles: boolean; importing: boolean }) {
  const l = layout();
  const active = dragging();
  const border = active ? c.green : "#BCCDB7";

  if (props.hasFiles) {
    return <View style={{ borderWidth: 1, borderStyle: "dashed", borderColor: active ? c.green : "#D3DECB", borderRadius: 10,
      backgroundColor: active ? "#E6F0E1" : "#F7FAF4", height: l.short ? 50 : 56, flexDirection: "row", alignItems: "center", gap: 10, paddingHorizontal: 14 }}>
      <Icon name="image" size={15} color="#93A88A" />
      <Text numberOfLines={1} style={{ flex: 1, fontSize: 11, color: active ? "#4E6B44" : "#8A9782" }}>
        {active ? "松开鼠标，添加图片或文件夹" : props.importing ? "正在读取图片…" : "继续拖入图片或文件夹"}
      </Text>
      <Button onPress={pickFiles} disabled={props.busy} hoverStyle={{ opacity: 0.7 }} disabledStyle={{ opacity: 0.4 }}
        style={{ cursor: "pointer", paddingVertical: 2, paddingHorizontal: 2 }}>
        <Text style={{ fontSize: 11, color: c.green, fontWeight: "600" }}>选择图片</Text>
      </Button>
    </View>;
  }

  return <View style={{ borderWidth: 1, borderStyle: "dashed", borderColor: border, borderRadius: 12,
    backgroundColor: active ? "#E6F0E1" : "#F5F8F1", height: l.short ? 156 : 208, alignItems: "center", justifyContent: "center", gap: 9 }}>
    <View style={{ flexDirection: "row", alignItems: "center", justifyContent: "center", marginBottom: 3 }}>
      <View style={{ width: 34, height: 38, marginRight: -8, borderRadius: 8, borderWidth: 1, borderColor: "#DFE6D9", backgroundColor: "#EBF0E5", transform: [{ rotate: "-14deg" }], alignItems: "center", justifyContent: "center" }}><Icon name="image" size={20} color="#9EB294" /></View>
      <View style={{ width: 42, height: 45, borderRadius: 9, borderWidth: 1, borderColor: "#D0DDC6", backgroundColor: "#FFFFFF", transform: [{ rotate: "8deg" }], alignItems: "center", justifyContent: "center", boxShadow: "0 4px 10px #4056300B" }}><Icon name="image" size={26} color="#6D905E" /></View>
    </View>
    <Text numberOfLines={1} style={{ fontSize: 15, fontWeight: "600", color: "#52674A" }}>{active ? "松开鼠标，添加图片或文件夹" : "拖入图片或文件夹，让轻盈开始"}</Text>
    <View style={{ flexDirection: "row", alignItems: "center", gap: 5 }}>
      <Text style={{ fontSize: 11, color: "#95A08D" }}>或</Text>
      <Button onPress={pickFiles} disabled={props.busy} hoverStyle={{ opacity: 0.7 }} disabledStyle={{ opacity: 0.4 }} style={{ cursor: "pointer", paddingVertical: 2 }}><Text style={{ fontSize: 11, color: c.green, fontWeight: "600" }}>{props.importing ? "正在读取图片…" : "选择图片"}</Text></Button>
      <Text style={{ fontSize: 11, color: "#95A08D" }}>或</Text>
      <Button onPress={pickFolders} disabled={props.busy} accessibilityLabel="选择文件夹" hoverStyle={{ opacity: 0.7 }} disabledStyle={{ opacity: 0.4 }} style={{ cursor: "pointer", paddingVertical: 2 }}><Text style={{ fontSize: 11, color: c.green, fontWeight: "600" }}>选择文件夹</Text></Button>
      <Text numberOfLines={1} style={{ flexShrink: 1, fontSize: 11, color: "#95A08D" }}>· 支持 PNG / JPEG</Text>
    </View>
  </View>;
}
