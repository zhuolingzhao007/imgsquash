import { Scroll, Text, View } from "nsh";
import { Button } from "nsh/base";
import { colors as c, Icon } from "../ui";
import type { IconName } from "../ui";
import { files, filter, totals } from "../state/queue";
import { RAIL_WIDTH, SIDEBAR_WIDTH } from "../state/layout";

type Filter = "all" | "pending" | "done";

function navigation() {
  return [
    { value: "all" as Filter, label: "全部图片", icon: "grid" as IconName, count: files().length },
    { value: "pending" as Filter, label: "待处理", icon: "clock" as IconName, count: files().filter(f => f.status !== "done").length },
    { value: "done" as Filter, label: "已完成", icon: "circleCheck" as IconName, count: totals().done },
  ];
}

function renderNavigation(props: { value: Filter; label: string; icon: IconName; count: number; active: boolean }) {
  return <Button onPress={() => filter.set(props.value)} selected={props.active}
    style={{ flexDirection: "row", alignItems: "center", gap: 10, height: 42, paddingHorizontal: 12, borderRadius: 8, cursor: "pointer", backgroundColor: props.active ? "#E2EBDD" : "transparent" }}
    hoverStyle={{ backgroundColor: "#E8EDE4" }} focusStyle={{ outlineWidth: 2, outlineColor: "#7AAA86" }}>
    <Icon name={props.icon} size={18} color={props.active ? c.green : c.muted} />
    <Text style={{ flex: 1, fontSize: 12, color: props.active ? c.greenDark : "#657168", fontWeight: props.active ? "600" : "400" }}>{props.label}</Text>
    <Text style={{ fontSize: 11, color: props.active ? c.green : "#8D968E" }}>{props.count}</Text>
  </Button>;
}

/**
 * 图标轨道：窗口不够放完整侧边栏时，只留图标，文字进 tooltip。
 *
 * logo 放在 Scroll 外面：它是身份标识，不该跟着导航一起滚走。
 */
function renderRail() {
  return <View style={{ width: RAIL_WIDTH, backgroundColor: c.sidebar, borderRightWidth: 1, borderColor: c.line }}>
    <View style={{ alignItems: "center", paddingTop: 25, paddingBottom: 16 }}>
      <View style={{ width: 36, height: 36, borderRadius: 11, backgroundColor: c.green, alignItems: "center", justifyContent: "center" }}>
        <Icon name="compress" size={25} color="#FFFFFF" />
      </View>
    </View>
    <Scroll bounces={false} style={{ flex: 1, minHeight: 0 }} contentStyle={{ flexGrow: 1, justifyContent: "space-between", alignItems: "center", paddingBottom: 25, gap: 12 }}>
      <View style={{ alignItems: "center", gap: 6 }}>
        {navigation().map(item => {
          const active = filter() === item.value;
          return <Button onPress={() => filter.set(item.value)} selected={active} accessibilityLabel={item.label} tooltip={`${item.label} · ${item.count}`}
            style={{ width: 42, height: 40, borderRadius: 9, alignItems: "center", justifyContent: "center", cursor: "pointer", backgroundColor: active ? "#E2EBDD" : "transparent" }}
            hoverStyle={{ backgroundColor: "#E8EDE4" }} focusStyle={{ outlineWidth: 2, outlineColor: "#7AAA86" }}>
            <Icon name={item.icon} size={18} color={active ? c.green : c.muted} />
          </Button>;
        })}
      </View>
      <View tooltip="离线可用" style={{ width: 6, height: 6, borderRadius: 3, backgroundColor: "#67935D" }} />
    </Scroll>
  </View>;
}

/**
 * 内容整体可滚动：高度不够时能滚到底，够高时上下两组各归其位。
 * logo 固定在顶部，只有导航和底部说明进 Scroll。
 */
export function renderSidebar(props: { rail?: boolean } = {}) {
  if (props.rail) return renderRail();
  const items = navigation();
  return <View style={{ width: SIDEBAR_WIDTH, backgroundColor: c.sidebar, borderRightWidth: 1, borderColor: c.line }}>
    <View style={{ flexDirection: "row", alignItems: "center", gap: 10, paddingHorizontal: 23, paddingTop: 25, paddingBottom: 24 }}>
      <View style={{ width: 36, height: 36, borderRadius: 11, backgroundColor: c.green, alignItems: "center", justifyContent: "center" }}><Icon name="compress" size={25} color="#FFFFFF" /></View>
      <View style={{ gap: 3 }}><Text style={{ fontSize: 17, fontWeight: "700", color: c.ink }}>ImgSquash</Text><Text style={{ fontSize: 9, letterSpacing: 2, color: c.muted }}>LOCAL STUDIO</Text></View>
    </View>
    <Scroll bounces={false} style={{ flex: 1, minHeight: 0 }} contentStyle={{ flexGrow: 1, justifyContent: "space-between", gap: 18, paddingHorizontal: 16, paddingBottom: 20 }}>
      <View>
        <Text style={{ fontSize: 10, color: "#969E95", paddingLeft: 12, marginBottom: 10, letterSpacing: 1 }}>工作空间</Text>
        <View style={{ gap: 5 }}>
          {items.map(item => renderNavigation({ ...item, active: filter() === item.value }))}
        </View>
      </View>
      <View style={{ gap: 19 }}>
        <View style={{ padding: 14, gap: 10, borderRadius: 10, backgroundColor: "#EAF0E5" }}>
          <Icon name="leaf" size={22} color="#6E8D60" />
          <Text style={{ fontSize: 12, fontWeight: "600", color: "#52684A" }}>小一点，刚刚好。</Text>
          <Text style={{ fontSize: 10, lineHeight: 17, color: "#7E8B75" }}>{"为你的每一张图片\n留住细节，减去负担。"}</Text>
        </View>
        <View style={{ flexDirection: "row", alignItems: "center", gap: 6, paddingLeft: 6 }}><View style={{ width: 5, height: 5, borderRadius: 3, backgroundColor: "#67935D" }} /><Text style={{ fontSize: 10, color: "#919A8B" }}>离线可用</Text><View style={{ flex: 1 }} /><Text style={{ fontSize: 10, color: "#A1A89C" }}>v1.0</Text></View>
      </View>
    </Scroll>
  </View>;
}
