import { signal, windowSize } from "nsh";

/** 三栏的固定宽度，和下面的断点一起算。 */
export const SIDEBAR_WIDTH = 190;
export const RAIL_WIDTH = 64;
export const SETTINGS_WIDTH = 256;

/**
 * 窗口的最小宽度。低于它布局不再继续压缩，直接按这个宽度排版——
 * 宁可横向裁掉，也不要把三栏挤成一团。
 */
export const MIN_WIDTH = 800;

/**
 * 窗口的最小高度。主区是纵向堆叠（标题 / 拖拽区 / 队列 / 底栏），
 * 再矮下去固定高度的几块放不开，表头会和底栏叠在一起。
 * 640 是空状态下刚好不重叠的下限；`short` 档位在 640–680 之间仍然生效。
 */
export const MIN_HEIGHT = 640;

/**
 * 根视图量到的真实尺寸。窗口尺寸在部分宿主里不会跟着变，量出来的才是
 * 实际可用空间，所以优先用它。
 */
export const surface = signal<{ width: number; height: number } | null>(null);

export function measureSurface(width: number, height: number) {
  const current = surface();
  if (current && Math.abs(current.width - width) < 0.5 && Math.abs(current.height - height) < 0.5) return;
  surface.set({ width, height });
}

export interface Layout {
  width: number;
  height: number;
  /** 设置面板停靠在右侧；否则收进抽屉。 */
  docked: boolean;
  /** 侧边栏：完整或图标轨道。 */
  sidebar: "full" | "rail";
  /** 主视图实际可用宽度。 */
  main: number;
  /** 主视图偏窄：列表收成两行、隐藏表头。 */
  compact: boolean;
  /** 窗口高度紧张：收起次要说明。 */
  short: boolean;
  /** 主视图左右留白。 */
  gutter: number;
}

/**
 * 一次算好所有布局档位，组件统一从这里取。
 *
 * 断点按**主视图宽度**算，而不是窗口宽度：侧边栏和停靠的设置面板都会吃掉
 * 宽度，只看窗口宽度会让「窗口不算窄但主视图已经很挤」的情况漏判。
 * 宽度先夹到 MIN_WIDTH，窗口再小也不会把布局压垮。
 */
export function layout(): Layout {
  const size = surface() ?? windowSize();
  const width = Math.max(MIN_WIDTH, size.width);
  const height = Math.max(MIN_HEIGHT, size.height);
  const docked = width >= 1080;
  const sidebar: Layout["sidebar"] = width >= 860 ? "full" : "rail";
  const sideWidth = sidebar === "full" ? SIDEBAR_WIDTH : RAIL_WIDTH;
  const main = width - sideWidth - (docked ? SETTINGS_WIDTH : 0);
  return {
    width,
    height,
    docked,
    sidebar,
    main,
    compact: main < 640,
    short: height < 680,
    gutter: main < 700 ? 24 : 32,
  };
}
