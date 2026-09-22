/** 窗口宽度档位:决定侧栏是否折叠、右栏是否收起。
 *
 *  侧栏(228px)与右栏(284px)都是固定宽度、不参与压缩,窄窗口时会把终端挤到
 *  只剩几十列。阈值按「终端至少剩多少列」倒推 —— 13px JetBrains Mono 的字符
 *  宽约 7.8px:
 *    wide   (>=1180):228 + 终端 + 284,终端约 112 列
 *    narrow (1040~1180):折叠侧栏省 164px,终端回到约 100 列
 *    tight  (<1040):再收起右栏省 284px,终端约 118 列
 *  tauri.conf.json 的 minWidth 是 980,所以 tight 是最窄的一档。
 */
export const BREAKPOINT_NARROW = 1180;
export const BREAKPOINT_TIGHT = 1040;

export type WidthTier = "wide" | "narrow" | "tight";

export function tierOf(width: number): WidthTier {
  if (width < BREAKPOINT_TIGHT) return "tight";
  if (width < BREAKPOINT_NARROW) return "narrow";
  return "wide";
}

/** 某个档位下,侧栏与右栏「应当」处于什么状态 */
export function layoutForTier(tier: WidthTier): { sidebarCollapsed: boolean; rightPanelOpen: boolean } {
  return {
    sidebarCollapsed: tier !== "wide",
    rightPanelOpen: tier !== "tight",
  };
}
