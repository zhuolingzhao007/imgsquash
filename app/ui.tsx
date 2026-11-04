import { Canvas, Path2D, Text, View } from "nsh";
import { Button } from "nsh/base";

export const colors = {
  ink: "#25372E", muted: "#7B857E", green: "#2E7D50", greenDark: "#246640",
  mint: "#EAF3EC", line: "#E6EAE5", canvas: "#FCFDFC", sidebar: "#F3F5F0",
};
const paths = {
  compress: "M4 4l6 6M4 10h6V4M20 20l-6-6m0 6v-6h6",
  image: "M5 3h14a2 2 0 0 1 2 2v14a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2ZM3 16l5-5 5 5 3-3 5 5M16 7h.01",
  grid: "M3 3h7v7H3ZM14 3h7v7h-7ZM3 14h7v7H3ZM14 14h7v7h-7Z",
  clock: "M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0ZM12 6v6l4 2",
  check: "M5 12l4 4L19 6",
  circleCheck: "M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0ZM7 12l3 3 7-7",
  plus: "M12 5v14M5 12h14",
  folder: "M3 7V5a2 2 0 0 1 2-2h5l2 3h7a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2ZM3 8h18",
  shield: "M12 2 3 6v6c0 5 9 10 9 10s9-5 9-10V6ZM8 12l3 3 5-6",
  close: "M6 6l12 12M18 6 6 18",
  settings: "M4 7h16M4 17h16M8 4v6M16 14v6",
  refresh: "M20 12a8 8 0 1 1-2.4-5.7M20 4v5h-5",
  leaf: "M20 3C8 1 2 8 5 15c3 7 15 7 15-12ZM4 21 15 10",
  info: "M22 12a10 10 0 1 1-20 0 10 10 0 0 1 20 0ZM12 11v6M12 7h.01",
};
export type IconName = keyof typeof paths;
export function Icon(props: { name: IconName; size?: number; color?: string }) {
  return <Canvas style={{ width: props.size ?? 20, height: props.size ?? 20, flexShrink: 0 }} draw={(ctx, size) => {
    ctx.scale(size.width / 24, size.height / 24);
    ctx.strokeStyle = props.color ?? colors.muted;
    ctx.lineWidth = 1.65; ctx.lineCap = "round"; ctx.lineJoin = "round";
    ctx.stroke(new Path2D(paths[props.name]));
  }} />;
}
export function Action(props: { label: string; icon?: IconName; onPress: () => void; primary?: boolean; disabled?: boolean; small?: boolean; testID?: string; accessibilityLabel?: string; tooltip?: string; iconOnly?: boolean; grow?: boolean }) {
  const height = props.small ? 34 : 42;
  return <Button testID={props.testID} onPress={props.onPress} disabled={props.disabled} accessibilityLabel={props.accessibilityLabel} tooltip={props.tooltip}
    style={{ flexDirection: "row", alignItems: "center", justifyContent: "center", gap: 8,
      width: props.iconOnly ? height : undefined, flex: props.grow ? 1 : undefined,
      paddingHorizontal: props.iconOnly ? 0 : props.small ? 12 : 18, height,
      borderRadius: 8, backgroundColor: props.primary ? colors.green : "#FFFFFF",
      borderWidth: props.primary ? 0 : 1, borderColor: colors.line, cursor: "pointer" }}
    hoverStyle={{ backgroundColor: props.primary ? colors.greenDark : "#F3F6F2" }}
    activeStyle={{ opacity: 0.8 }} focusStyle={{ outlineWidth: 2, outlineColor: "#7AAA86" }}
    disabledStyle={{ opacity: 0.4, cursor: "default" }}>
    {props.icon && <Icon name={props.icon} size={16} color={props.primary ? "#FFFFFF" : colors.ink} />}
    {!props.iconOnly && <Text numberOfLines={1} style={{ fontSize: 12, fontWeight: "600", color: props.primary ? "#FFFFFF" : colors.ink }}>{props.label}</Text>}
  </Button>;
}
export function Pill(props: { text: string; green?: boolean }) {
  return <View style={{ borderRadius: 5, backgroundColor: props.green ? colors.mint : "#F0F2EF", paddingHorizontal: 7, paddingVertical: 4 }}>
    <Text style={{ fontSize: 10, fontWeight: "500", color: props.green ? colors.green : colors.muted }}>{props.text}</Text>
  </View>;
}
