/** 窗口宽度档位自检:阈值划分,以及「跨档才动作」这条行为。
 *
 *  背景:侧栏(228)与右栏(284)都是固定宽度、不参与压缩,窄窗口会把终端挤到
 *  只剩几十列。自动折叠能救这一点,但 naive 的做法(每次 resize 都套用该档
 *  布局)会让用户每次手动展开侧栏都被下一次 resize 打回去 —— 所以只在跨档
 *  的那一刻动作。下面第 3 组就是钉这条。 */
import { BREAKPOINT_NARROW, BREAKPOINT_TIGHT, layoutForTier, tierOf } from "./breakpoint.ts";

let failed = 0;
function check(cond: boolean, msg: string) {
  if (!cond) {
    console.error("FAIL: " + msg);
    failed++;
  }
}

// ---- 1. 阈值划分(边界值本身归上一档) ----
check(tierOf(1440) === "wide", "默认 1440 属 wide");
check(tierOf(BREAKPOINT_NARROW) === "wide", `边界 ${BREAKPOINT_NARROW} 本身仍属 wide`);
check(tierOf(BREAKPOINT_NARROW - 1) === "narrow", `${BREAKPOINT_NARROW - 1} 落入 narrow`);
check(tierOf(BREAKPOINT_TIGHT) === "narrow", `边界 ${BREAKPOINT_TIGHT} 本身仍属 narrow`);
check(tierOf(BREAKPOINT_TIGHT - 1) === "tight", `${BREAKPOINT_TIGHT - 1} 落入 tight`);
check(tierOf(980) === "tight", "minWidth 980 属 tight");

// ---- 2. 各档位的目标布局 ----
const wide = layoutForTier("wide");
check(!wide.sidebarCollapsed && wide.rightPanelOpen, "wide:侧栏展开、右栏打开");

const narrow = layoutForTier("narrow");
check(narrow.sidebarCollapsed, "narrow:侧栏折叠");
check(narrow.rightPanelOpen, "narrow:右栏仍打开 —— 折叠侧栏已省出 164px,不必连右栏一起收");

const tight = layoutForTier("tight");
check(tight.sidebarCollapsed && !tight.rightPanelOpen, "tight:侧栏折叠且右栏收起");

// ---- 3. 跨档才动作:档内的 resize 不能覆盖用户的手动操作 ----
{
  let tier = tierOf(1100); // narrow
  let sidebarCollapsed = layoutForTier(tier).sidebarCollapsed;
  check(sidebarCollapsed, "进入 narrow 时自动折叠");

  sidebarCollapsed = false; // 用户手动展开

  // 档内继续拖动窗口(仍在 1040~1180 区间)
  for (const w of [1080, 1120, 1179]) {
    const next = tierOf(w);
    if (next !== tier) {
      tier = next;
      sidebarCollapsed = layoutForTier(next).sidebarCollapsed;
    }
  }
  check(!sidebarCollapsed, "档内继续 resize 不覆盖用户的手动展开");

  // 跨回 wide 才重新套用该档布局
  const next = tierOf(1300);
  check(next !== tier, "1300 已跨出 narrow");
  tier = next;
  sidebarCollapsed = layoutForTier(next).sidebarCollapsed;
  check(!sidebarCollapsed, "回到 wide:侧栏恢复展开");
}

if (failed > 0) {
  console.error(`断点自检未通过:${failed} 项`);
  process.exit(1);
}
console.log("BREAKPOINT_PASS");
