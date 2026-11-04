import { Text, View } from "nsh";
import { colors as c } from "../ui";

export function renderStat(props: { label: string; value: string }) {
  return <View style={{ flexDirection: "row", alignItems: "center", justifyContent: "space-between" }}>
    <Text style={{ fontSize: 12, color: c.muted }}>{props.label}</Text>
    <Text style={{ fontSize: 12, fontWeight: "500", color: c.ink }}>{props.value}</Text>
  </View>;
}
