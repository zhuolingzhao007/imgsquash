import { signal, Storage } from "nsh";

export type Mode = "smart" | "lossless";
export type Preset = "standard" | "strict" | "smaller";
/** 原格式或 WebP；AVIF 暂无回读验证能力。 */
export type OutputFormat = "keep" | "webp";

/** 档位表达取舍，不再显示所有图片共用的感知分数线。 */
export const presets: Record<Preset, { label: string; hint: string; tip: string }> = {
  standard: { label: "标准", hint: "自适应均衡", tip: "根据实际节省与画质损失选择结果" },
  strict: { label: "严格", hint: "画质优先", tip: "对相同的画质损失要求更多体积收益" },
  smaller: { label: "更小", hint: "体积优先", tip: "允许更多失真换取更小体积" },
};

export const mode = signal<Mode>("smart");
export const preset = signal<Preset>("standard");
export const outputFormat = signal<OutputFormat>("keep");

/** 格式转换要重新编码，和「无损」冲突，所以无损模式下不提供。 */
export function formatConversionAvailable() {
  return mode() === "smart";
}

/** 保留智能模式的格式偏好，但无损模式的有效格式始终跟随原图。 */
export function effectiveOutputFormat(): OutputFormat {
  return mode() === "lossless" ? "keep" : outputFormat();
}

export function setMode(value: Mode) {
  mode.set(value);
  Storage.setItem("tinypng.mode", value).catch(() => {});
}

export function setPreset(value: Preset) {
  preset.set(value);
  Storage.setItem("tinypng.preset", value).catch(() => {});
}

export function setOutputFormat(value: OutputFormat) {
  outputFormat.set(value);
  Storage.setItem("tinypng.outputFormat", value).catch(() => {});
}

void (async () => {
  try {
    const savedMode = await Storage.getItem("tinypng.mode");
    if (savedMode === "smart" || savedMode === "lossless") mode.set(savedMode);
    const savedPreset = await Storage.getItem("tinypng.preset");
    if (savedPreset === "standard" || savedPreset === "strict" || savedPreset === "smaller") preset.set(savedPreset);
    const savedOutputFormat = await Storage.getItem("tinypng.outputFormat");
    if (savedOutputFormat === "keep" || savedOutputFormat === "webp") outputFormat.set(savedOutputFormat);
  } catch {
    // 存储不可用时用默认值。
  }
})();
