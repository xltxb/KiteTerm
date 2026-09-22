import { create } from "zustand";
import { ipc } from "@/ipc";
import { retheme } from "@/stores/session";
import { DEFAULT_SETTINGS, type AppSettings, type Screen } from "@/types";
import { layoutForTier, tierOf, type WidthTier } from "@/lib/breakpoint";

interface UiState {
  screen: Screen;
  paletteOpen: boolean;
  sidebarCollapsed: boolean;
  rightPanelOpen: boolean;
  /** 上一次应用过的宽度档位 —— 只在跨档时自动调整,档内尊重用户的手动操作 */
  widthTier: WidthTier;
  rightTab: "snip" | "rec" | "ai";
  settings: AppSettings;
  go: (screen: Screen) => void;
  setPalette: (open: boolean) => void;
  toggleSidebar: () => void;
  toggleRightPanel: () => void;
  applyWidth: (width: number) => void;
  setRightTab: (tab: "snip" | "rec" | "ai") => void;
  setSettings: (patch: Partial<AppSettings>) => void;
  loadSettings: () => Promise<void>;
}

function applyTheme(theme: "dark" | "light") {
  document.documentElement.setAttribute("data-theme", theme);
  // 终端的配色是创建时快照的,不会跟着 data-theme 走 —— 属性落到 DOM 之后
  // 立刻把存活的实例重新上色(此时 getComputedStyle 才拿得到新一档的值)
  retheme();
}

export const useUiStore = create<UiState>((set, get) => ({
  screen: "terminal",
  paletteOpen: false,
  sidebarCollapsed: false,
  rightPanelOpen: true,
  widthTier: "wide",
  rightTab: "snip",
  settings: DEFAULT_SETTINGS,
  go: (screen) => set({ screen, paletteOpen: false }),
  setPalette: (open) => set({ paletteOpen: open }),
  toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
  toggleRightPanel: () => set((s) => ({ rightPanelOpen: !s.rightPanelOpen })),
  /** 窗口尺寸变化时调用。只在跨档的那一刻套用该档的布局 —— 档内不动,
   *  否则用户每次手动展开侧栏都会被下一次 resize 打回去。 */
  applyWidth: (width) => {
    const tier = tierOf(width);
    if (tier === get().widthTier) return;
    set({ widthTier: tier, ...layoutForTier(tier) });
  },
  setRightTab: (tab) => set({ rightTab: tab }),
  setSettings: (patch) => {
    const settings = { ...get().settings, ...patch };
    set({ settings });
    applyTheme(settings.theme);
    void ipc.saveSettings(settings);
  },
  loadSettings: async () => {
    const settings = await ipc.getSettings();
    set({ settings });
    applyTheme(settings.theme);
  },
}));
