import { expect, test } from "nsh/test";
import { files, fileDetail, formatOf, formatSize, methodLabel, ratioLabel, totals } from "./compression";
import type { FileItem } from "./compression";
import { effectiveOutputFormat, mode, outputFormat } from "./state/settings";
import { fileDetailCompact } from "./state/queue";

function item(patch: Partial<FileItem>): FileItem {
  return {
    path: "/tmp/a.png", name: "a.png", format: "PNG",
    inputSize: 0, outputSize: 0, ratio: 0, status: "idle",
    method: "", score: 100, lossy: false, keptIcc: false, ...patch,
  };
}

test("lossless mode uses the original format while remembering the smart preference", () => {
  mode.set("smart"); outputFormat.set("webp");
  expect(effectiveOutputFormat()).toBe("webp");
  mode.set("lossless");
  expect(effectiveOutputFormat()).toBe("keep");
  expect(outputFormat()).toBe("webp");
  mode.set("smart");
  expect(effectiveOutputFormat()).toBe("webp");
  outputFormat.set("keep");
});

test("labels match the text shown in the queue", () => {
  expect(formatOf("icon.PNG")).toBe("PNG");
  expect(formatOf("photo.JPEG")).toBe("JPEG");
  expect(methodLabel("quantizr-c128")).toBe("128 色");
  expect(methodLabel("quantizr-c256")).toBe("256 色");
  expect(methodLabel("mozjpeg-q80")).toBe("q80");
  expect(methodLabel("webp")).toBe("WebP 无损");
  expect(methodLabel("oxipng")).toBe("无损");
  expect(methodLabel("oxipng-16")).toBe("无损 · 16 位深");
  expect(methodLabel("original")).toBe("原样");
  expect(methodLabel("kept")).toBe("已最优");
  expect(formatSize(512)).toBe("512 B");
  expect(formatSize(1536)).toBe("1.5 KB");
});

test("file rows explain each state", () => {
  expect(fileDetail(item({ status: "idle" }))).toBe("PNG · 保持原始尺寸");
  expect(fileDetail(item({ status: "compressing" }))).toBe("PNG · 保持原始尺寸");
  expect(fileDetail(item({ status: "done", method: "quantizr-c256", lossy: true, score: 88.8 }))).toBe("PNG · 256 色 · 感知 88.8");
  expect(fileDetail(item({ status: "done", method: "oxipng" }))).toBe("PNG · 无损");
  expect(fileDetail(item({ status: "done", method: "quantizr-c256", lossy: true, score: 91.2, keptIcc: true }))).toBe("PNG · 256 色 · 感知 91.2 · 保留 ICC");
  expect(fileDetail(item({ status: "done", method: "original", format: "JPEG" }))).toBe("JPEG · 原样");
  expect(fileDetail(item({ status: "error", error: "解码失败" }))).toBe("解码失败");
  expect(ratioLabel(item({ status: "done", ratio: 60 }))).toBe("−60%");
  expect(ratioLabel(item({ status: "done", method: "original" }))).toBe("原样");
  expect(ratioLabel(item({ status: "done", method: "kept" }))).toBe("无收益");
  expect(ratioLabel(item({ status: "done", method: "oxipng" }))).toBe("已优化");
});

test("totals only count completed files", () => {
  files.set([
    item({ path: "/a.png", status: "done", inputSize: 1000, outputSize: 400, ratio: 60, method: "quantizr-c128", lossy: true }),
    item({ path: "/b.jpg", name: "b.jpg", format: "JPEG", status: "done", inputSize: 2000, outputSize: 2000, method: "original" }),
    item({ path: "/c.png", status: "idle", inputSize: 500 }),
  ]);
  const result = totals();
  expect(result.done).toBe(2);
  expect(result.before).toBe(3000);
  expect(result.after).toBe(2400);
  expect(result.saved).toBe(600);
  expect(result.ratio).toBe(20);
});

test("large-image rows distinguish tiled validation from the global score", () => {
  const file = item({ status: "done", method: "stream-jpeg-q80", lossy: true, score: 90.1, scoreKind: "tiled" });
  expect(methodLabel("stream-png-lossless")).toBe("无损 · 低内存");
  expect(methodLabel("stream-png-c128")).toBe("128 色 · 低内存");
  expect(fileDetail(file)).toBe("PNG · q80 · 低内存 · 分块最低 90.1");
  expect(fileDetailCompact(file)).toBe("0 B → 0 B · q80 · 低内存 · 分块最低 90.1");
});
