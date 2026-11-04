import { Scroll, Text, View } from "nsh";
import { Button } from "nsh/base";
import { colors as c, Icon } from "../ui";
import type { IconName } from "../ui";
import { formatDuration, formatSize, spentMs, totals } from "../state/queue";
import { effectiveOutputFormat, mode, preset, presets, setMode, setPreset, setOutputFormat, formatConversionAvailable } from "../state/settings";
import type { Mode, Preset, OutputFormat } from "../state/settings";
import { layout, SETTINGS_WIDTH } from "../state/layout";
import { settingsOpen } from "../state/view";

const LABEL: Record<string, string> = { keep: "跟随原图", webp: "WebP" };

function renderModeCard(props: { value: Mode; active: boolean; busy: boolean; icon: IconName; title: string; description: string; tip: string }) {
  const active = props.active;
  return <Button onPress={() => !props.busy && setMode(props.value)} disabled={props.busy} accessibilityLabel={props.title} tooltip={props.tip}
    style={{ flexDirection: "column", alignItems: "stretch", borderWidth: 1, borderColor: active ? "#B4CBA8" : c.line, borderRadius: 9,
      paddingHorizontal: 10, paddingVertical: 9, backgroundColor: active ? "#F0F6EB" : "#FFFFFF", gap: 4, cursor: "pointer" }}
    hoverStyle={{ backgroundColor: active ? "#F0F6EB" : "#F4F7F1" }} disabledStyle={{ opacity: 0.5 }}>
    <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
      <Icon name={props.icon} size={15} color={active ? c.green : "#8D9A85"} />
      <Text style={{ flex: 1, fontSize: 12, fontWeight: "600", color: active ? c.greenDark : c.ink }}>{props.title}</Text>
      {active && <Icon name="circleCheck" size={13} color={c.green} />}
    </View>
    <Text numberOfLines={1} style={{ fontSize: 9.5, lineHeight: 14, color: active ? "#819177" : "#9AA290" }}>{props.description}</Text>
  </Button>;
}

function renderChoice(props: { active: boolean; busy: boolean; disabled?: boolean; label: string; hint: string; tip: string; onPress: () => void }) {
  const active = props.active;
  const locked = props.disabled === true;
  return <Button onPress={() => !props.busy && !locked && props.onPress()} disabled={props.busy || locked} accessibilityLabel={props.label} tooltip={props.tip}
    style={{ flex: 1, flexDirection: "column", paddingVertical: 6, borderRadius: 7, alignItems: "center", gap: 2,
      backgroundColor: active ? c.green : "#FFFFFF", borderWidth: 1, borderColor: active ? c.green : c.line, cursor: "pointer" }}
    hoverStyle={{ backgroundColor: active ? c.greenDark : "#F3F6F2" }} disabledStyle={{ opacity: locked ? 0.4 : 0.5 }}>
    <Text numberOfLines={1} style={{ fontSize: 10, fontWeight: "600", color: active ? "#FFFFFF" : c.ink }}>{props.label}</Text>
    <Text numberOfLines={1} style={{ fontSize: 8, color: active ? "#DCEBE0" : "#9AA290" }}>{props.hint}</Text>
  </Button>;
}

function renderSavings() {
  const t = totals();
  return <View style={{ padding: 13, borderWidth: 1, borderColor: "#E4E9DD", backgroundColor: "#F0F4E9", borderRadius: 11, gap: 9 }}>
    <View style={{ flexDirection: "row", alignItems: "center", gap: 7 }}>
      <Icon name="compress" size={14} color="#738966" />
      <Text style={{ fontSize: 11, color: "#75846B" }}>本次已节省</Text>
    </View>
    <Text style={{ fontSize: 24, fontWeight: "600", letterSpacing: -1, color: "#4F7340" }}>{t.done ? formatSize(t.saved) : "0 B"}</Text>
    {/* 用 flexGrow 按比例分两段：百分比宽度在 flex 容器里解析不出来，
        填充条会退化成固定宽度、还和轨道上下排成两条。 */}
    <View style={{ height: 4, borderRadius: 2, overflow: "hidden", flexDirection: "row", backgroundColor: "#DEE6D5" }}>
      <View style={{ flexGrow: t.ratio, flexBasis: 0, backgroundColor: "#7F9C67" }} />
      <View style={{ flexGrow: 100 - t.ratio, flexBasis: 0 }} />
    </View>
    <Text numberOfLines={2} style={{ fontSize: 10, lineHeight: 15, color: "#94A187" }}>{t.done ? `已完成 ${t.done} 张 · 体积减少 ${t.ratio}%${spentMs() ? ` · 用时 ${formatDuration(spentMs())}` : ""}` : "每一份轻盈，都会在这里看见。"}</Text>
  </View>;
}

/**
 * 设置内容整体可滚动：窗口一矮，靠滚动兜底，而不是把底部裁掉。
 * 内容不足一屏时 `space-between` 把统计卡片推到底，和原来贴底的效果一致。
 */
function renderBody(props: { busy: boolean }) {
  const l = layout();
  return <Scroll bounces={false} style={{ flex: 1, minHeight: 0 }}
    contentStyle={{ flexGrow: 1, justifyContent: "space-between", gap: 14, paddingHorizontal: 18, paddingTop: 4, paddingBottom: 16 }}>
    <View style={{ gap: 13 }}>
      <View style={{ gap: 7 }}>
        <Text style={{ fontSize: 12, color: c.muted }}>压缩方式</Text>
        {renderModeCard({ value: "smart", active: mode() === "smart", busy: props.busy, icon: "compress", title: "智能压缩", description: "按每张图的体积与画质自动选择。", tip: "按每张图的体积与画质自动选择。" })}
        {renderModeCard({ value: "lossless", active: mode() === "lossless", busy: props.busy, icon: "leaf", title: "无损优化", description: "PNG 无损重压缩；JPEG 原样保留。", tip: "PNG 只做无损重压缩；JPEG 原样保留。" })}
      </View>

      {mode() === "smart" && <View style={{ gap: 7 }}>
        <Text style={{ fontSize: 12, color: c.muted }}>质量档位</Text>
        <View style={{ flexDirection: "row", gap: 6 }}>
          {(Object.keys(presets) as Preset[]).map(key => renderChoice({ active: preset() === key, busy: props.busy,
            label: presets[key].label, hint: presets[key].hint, tip: presets[key].tip, onPress: () => setPreset(key) }))}
        </View>
        {!l.short && <Text style={{ fontSize: 9, color: "#9AA290", lineHeight: 13 }}>越严格，画质越接近原图，体积也会大一点。</Text>}
      </View>}

      <View style={{ gap: 7 }}>
        <Text style={{ fontSize: 12, color: c.muted }}>输出格式</Text>
        <View style={{ flexDirection: "row", gap: 6 }}>
          {(["keep", "webp"] as OutputFormat[]).map(fmt => renderChoice({ active: effectiveOutputFormat() === fmt, busy: props.busy,
            // 换格式必须重新编码，和无损模式的承诺冲突，所以只留「跟随原图」可选。
            disabled: !formatConversionAvailable() && fmt !== "keep",
            label: LABEL[fmt], hint: effectiveOutputFormat() === fmt ? "启用" : fmt === "keep" ? "保持原格式" : "转换格式", tip: fmt === "keep" ? "保持原格式" : `转换为 ${LABEL[fmt]}`,
            onPress: () => setOutputFormat(fmt) }))}
        </View>
        {!l.short && <Text style={{ fontSize: 9, color: "#9AA290", lineHeight: 13 }}>{formatConversionAvailable()
          ? "比较 WebP 有损与无损结果，按实际收益选择。"
          : "无损模式不做格式转换：换格式必须重新编码。"}</Text>}
      </View>

      <View style={{ height: 1, backgroundColor: c.line }} />

      <View style={{ gap: 8 }}>
        <Text style={{ fontSize: 12, color: c.muted }}>保存位置</Text>
        <View style={{ flexDirection: "row", alignItems: "center", gap: 8, padding: 9, borderWidth: 1, borderColor: c.line, borderRadius: 8, backgroundColor: "#FFFFFF" }}>
          <Icon name="folder" size={15} color="#88947D" /><Text numberOfLines={1} style={{ fontSize: 11, color: "#5A6752" }}>compressed /</Text>
        </View>
        {!l.short && <Text style={{ fontSize: 9, color: "#9AA290", lineHeight: 13 }}>自动保存到原图所在文件夹的 compressed 子文件夹。</Text>}
        <View style={{ flexDirection: "row", alignItems: "center", gap: 6 }}><Icon name="check" size={12} color={c.green} /><Text style={{ fontSize: 10, color: "#6B815F" }}>始终保留原图</Text></View>
      </View>
    </View>

    <View style={{ gap: 11 }}>
      {renderSavings()}
      <View style={{ flexDirection: "row", alignItems: "center", gap: 6, justifyContent: "center" }}><Icon name="shield" size={12} color="#99A38F" /><Text style={{ fontSize: 10, color: "#99A38F" }}>图片留在本机，无需上传</Text></View>
    </View>
  </Scroll>;
}

function renderHeader(props: { drawer: boolean }) {
  return <View style={{ flexDirection: "row", alignItems: "center", gap: 8, paddingHorizontal: 18, paddingTop: 16, paddingBottom: 10 }}>
    <Icon name="settings" size={17} color="#6B7862" />
    <Text style={{ flex: 1, fontSize: 13, fontWeight: "600", color: c.ink }}>压缩设置</Text>
    {props.drawer && <Button onPress={() => settingsOpen.set(false)} accessibilityLabel="关闭设置" tooltip="关闭设置"
      style={{ padding: 5, borderRadius: 6, cursor: "pointer" }} hoverStyle={{ backgroundColor: "#EDF0E9" }}>
      <Icon name="close" size={14} color="#8A9584" />
    </Button>}
  </View>;
}

/** 停靠时是第三栏，否则是右侧抽屉（由 App 决定何时渲染）。 */
export function renderSettings(props: { busy: boolean; drawer?: boolean }) {
  const drawer = props.drawer === true;
  return <View style={drawer
    ? { position: "absolute", top: 0, right: 0, bottom: 0, width: Math.min(304, layout().width - 40), zIndex: 30, backgroundColor: "#F9FAF7", borderLeftWidth: 1, borderColor: "#DDE4D8", boxShadow: "-12px 0 32px #25372E14" }
    : { width: SETTINGS_WIDTH, borderLeftWidth: 1, borderColor: c.line, backgroundColor: "#F9FAF7" }}>
    {renderHeader({ drawer })}
    {renderBody({ busy: props.busy })}
  </View>;
}
