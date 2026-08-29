import { create } from "zustand";
import { ipc } from "@/ipc";
import { retheme } from "@/stores/session";
import { DEFAULT_SETTINGS, type AppSettings, type Screen } from "@/types";

interface UiState {
  screen: Screen;
  paletteOpen: boolean;
  sidebarCollapsed: boolean;
  rightPanelOpen: boolean;
  rightTab: "snip" | "rec" | "ai";
  settings: AppSettings;
  go: (screen: Screen) => void;
  setPalette: (open: boolean) => void;
  toggleSidebar: () => void;
  toggleRightPanel: () => void;
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
  rightTab: "snip",
  settings: DEFAULT_SETTINGS,
  go: (screen) => set({ screen, paletteOpen: false }),
  setPalette: (open) => set({ paletteOpen: open }),
  toggleSidebar: () => set((s) => ({ sidebarCollapsed: !s.sidebarCollapsed })),
  toggleRightPanel: () => set((s) => ({ rightPanelOpen: !s.rightPanelOpen })),
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
