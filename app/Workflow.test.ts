import { expect, open, test } from "nsh/test";

test("folder import, repeated clicks and recompression preserve originals and previous outputs", async () => {
  // 文件操作在桌面 App 自己的权限上下文执行，不扩大测试运行器或正式 App 的权限。
  const app = await open("./WorkflowApp.tsx", { platform: "macos", storage: {} });
  await app.press("运行验收");
  await app.waitUntil(() => app.texts().some(text => text.startsWith("验收通过") || text.startsWith("验收失败")), { timeout: 25_000 });
  expect(app.texts()).toContain("验收通过");
});
