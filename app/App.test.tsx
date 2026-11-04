import { expect, open, test } from "nsh/test";

test("empty workspace and queue filters", async () => {
  const app = await open("./App.tsx", { width: 1180, height: 780, platform: "macos", storage: {} });
  expect(app.texts()).toContain("让图片轻一点。");
  expect(app.enabled("id:compress")).toBe(false);
  await app.press("已完成");
  expect(app.texts()).toContain("完成的图片会显示在这里");
  await app.press("全部图片");
  expect(app.texts()).toContain("还没有图片，试着添加第一张吧");
  expect(app.shows("打开文件夹")).toBe(false);
  await app.setViewport(820, 720);
  expect(app.shows("添加图片")).toBe(true);
  expect(app.enabled("id:compress")).toBe(false);
});

test("compression modes switch quality controls", async () => {
  const app = await open("./App.tsx", { width: 1180, height: 780, platform: "macos", storage: {} });
  expect(app.texts()).toContain("智能压缩");
  expect(app.texts()).toContain("自适应均衡");
  await app.press("无损优化");
  expect(app.texts()).toContain("跟随原图");
  expect(app.texts()).not.toContain("自适应均衡");
  await app.press("智能压缩");
  await app.press("严格");
  expect(app.texts()).toContain("画质优先");
});

test("remembers the last compression mode", async () => {
  const app = await open("./App.tsx", {
    width: 1180, height: 780, platform: "macos",
    storage: { "tinypng.mode": "lossless", "tinypng.preset": "strict" },
  });
  await app.wait(100);
  expect(app.texts()).toContain("跟随原图");
  expect(app.texts()).not.toContain("自适应均衡");
});

test("the drop zone leaves tabs that only show results", async () => {
  const app = await open("./App.tsx", { width: 1180, height: 780, platform: "macos", storage: {} });
  expect(app.shows("选择文件夹")).toBe(true);
  expect(app.shows("添加文件夹")).toBe(true);
  await app.press("待处理");
  expect(app.shows("选择文件夹")).toBe(false);
  await app.press("已完成");
  expect(app.shows("选择文件夹")).toBe(false);
});

test("narrow windows keep the rail and the settings drawer", async () => {
  const app = await open("./App.tsx", { width: 560, height: 640, platform: "macos", storage: {} });
  // 侧边栏退成图标轨道，筛选靠轨道的无障碍标签；设置收进抽屉。
  expect(app.shows("工作空间")).toBe(false);
  expect(app.shows("全部图片")).toBe(true);
  expect(app.shows("压缩方式")).toBe(false);
  await app.press("已完成");
  expect(app.texts()).toContain("完成的图片会显示在这里");
  await app.press("压缩设置");
  expect(app.texts()).toContain("压缩方式");
  await app.press("关闭设置");
  expect(app.shows("压缩方式")).toBe(false);
});

test("the layout never goes below the minimum width", async () => {
  // 480 的窗口按 800 排版：完整侧边栏不会退化成 tab，也不会挤成一团。
  const app = await open("./App.tsx", { width: 480, height: 640, platform: "macos", storage: {} });
  expect(app.shows("工作空间")).toBe(false);
  expect(app.shows("全部图片")).toBe(true);
  expect(app.shows("压缩设置")).toBe(true);
  expect(app.shows("添加图片")).toBe(true);
});

test("widening the window docks the settings panel again", async () => {
  const app = await open("./App.tsx", { width: 900, height: 780, platform: "macos", storage: {} });
  expect(app.shows("压缩方式")).toBe(false);
  await app.press("压缩设置");
  expect(app.texts()).toContain("压缩方式");
  await app.setViewport(1180, 780);
  await app.wait(100);
  expect(app.texts()).toContain("压缩方式");
  expect(app.shows("关闭设置")).toBe(false);
});

test("a short window keeps the settings reachable", async () => {
  const app = await open("./App.tsx", { width: 1180, height: 520, platform: "macos", storage: {} });
  expect(app.texts()).toContain("压缩方式");
  expect(app.texts()).toContain("保存位置");
  // 矮窗口里次要说明让位，正文不再被裁掉。
  expect(app.texts()).not.toContain("越严格，画质越接近原图，体积也会大一点。");
});

test("checking files reports the queue matches the disk", async () => {
  const app = await open("./App.tsx", { width: 1180, height: 780, platform: "macos", storage: {} });
  await app.press("检查文件状态");
  await app.wait(100);
  // 空队列没什么可对账的，不能回一句「文件都在」。
  expect(app.texts()).toContain("队列是空的，先添加图片再检查。");
  expect(app.texts()).not.toContain("文件都在，状态正常。");
});

/**
 * 以 Windows 的身份跑一遍：不需要 Windows 机器，也能挡住「一上来就崩」的问题
 * （平台分支写错、Windows 独有的取值、布局在 Windows 断点下炸掉）。
 * 真机才有的东西（原生文件面板、资源管理器、窗口按钮位置）这里测不到。
 *
 * 注意：模拟平台只影响 nsh 自己的接口（keymap.label、platform.os 等）。
 * `os.platform()` 读的是宿主系统，在 macOS 上永远返回 "macos"，别拿它做分支判断。
 */
test("opens as Windows without breaking the shell", async () => {
  const app = await open("./App.tsx", { width: 1180, height: 780, platform: "windows", storage: {} });
  expect(app.texts()).toContain("让图片轻一点。");
  expect(app.texts()).toContain("开始压缩");
  expect(app.enabled("id:compress")).toBe(false);
  // 1180 宽时设置面板停靠，不用点就可见。
  expect(app.texts()).toContain("压缩方式");
  // 窄窗口的断点和 macOS 一致：侧边栏退成轨道，设置进抽屉。
  await app.setViewport(820, 720);
  await app.wait(100);
  expect(app.shows("添加图片")).toBe(true);
  expect(app.shows("压缩方式")).toBe(false);
  await app.press("压缩设置");
  expect(app.texts()).toContain("压缩方式");
});
