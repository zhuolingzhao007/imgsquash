import { Text, View } from "nsh";
import { Button } from "nsh/base";
import { titleBar } from "nsh/desktop";
import { colors as c, Icon } from "../ui";
import { layout } from "../state/layout";

/**
 * 顶栏只留拖拽区和右侧的状态。品牌名交给侧边栏，避免同一行里出现两遍。
 */
export function renderTopBar(props: { onOpenSettings: () => void; settingsOpen: boolean }) {
  const l = layout();
  return <View desktop={{ windowDrag: true }} style={{ flexDirection: "row", alignItems: "center", height: Math.max(titleBar().height, 48), paddingLeft: Math.max(titleBar().left, 84), paddingRight: 20, gap: 12, borderBottomWidth: 1, borderColor: c.line, backgroundColor: "#F5F7F2" }}>
    <View style={{ flex: 1 }} />
    {!l.docked && <Button onPress={props.onOpenSettings} accessibilityLabel={props.settingsOpen ? "收起压缩设置" : "压缩设置"} tooltip="压缩设置"
      style={{ flexDirection: "row", alignItems: "center", gap: 6, height: 26, paddingHorizontal: 9, borderRadius: 7, cursor: "pointer", backgroundColor: props.settingsOpen ? "#E4EAE0" : "transparent" }}
      hoverStyle={{ backgroundColor: "#E9EDE4" }} focusStyle={{ outlineWidth: 2, outlineColor: "#7AAA86" }}>
      <Icon name="settings" size={15} color="#6B7862" />
      <Text style={{ fontSize: 11, color: "#6B7862" }}>压缩设置</Text>
    </Button>}
    <View style={{ flexDirection: "row", alignItems: "center", gap: 6 }}>
      <Icon name="shield" size={12} color="#849778" />
      <Text style={{ fontSize: 10, color: "#849778" }}>本地图片压缩工具</Text>
    </View>
  </View>;
}
