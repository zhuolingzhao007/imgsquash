import { Text, View } from "nsh";
import { colors as c, Pill } from "../ui";
import { layout } from "../state/layout";

export function renderPageHeader() {
  const l = layout();
  const size = l.short ? 22 : 25;
  // 大字号必须给 lineHeight：只靠 fontSize 时这一行的行高会被算成 0，整段文字看不见。
  const line = Math.round(size * 1.35);
  return <View style={{ marginBottom: l.short ? 12 : 16 }}>
    <View style={{ flexDirection: "row", alignItems: "center", gap: 12, height: line }}>
      <Text style={{ flex: 1, fontSize: size, lineHeight: line, fontWeight: "600", color: c.ink }}>让图片轻一点。</Text>
      <Pill text="100% 本地处理" green />
    </View>
    {!l.short && <Text numberOfLines={2} style={{ fontSize: 11, lineHeight: 17, color: "#99A18F", marginTop: 4 }}>更小的体积，同样的细节。简单、安全地优化你的图片。</Text>}
  </View>;
}
