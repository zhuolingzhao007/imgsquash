import { signal } from "nsh";

export const dragging = signal(false);

/** 窄窗口下设置抽屉是否展开；设置面板停靠时不用它。 */
export const settingsOpen = signal(false);
