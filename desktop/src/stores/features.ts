import { create } from "zustand";
import { ipc } from "@/ipc";
import type { Features } from "@/types";

/** 从未拉到过时的兜底:全关。与 Rust 侧 Features::unknown 保持一致。 */
const UNKNOWN: Features = {
  registration: "closed",
  login: false,
  sync: false,
  ai: false,
  passwordMinLength: 8,
  passwordRule: "",
  known: false,
};

interface FeaturesState {
  features: Features;
  /** 启动时先读缓存(同步、无网络),再后台向平台刷新一次 */
  load: () => Promise<void>;
  refresh: () => Promise<void>;
}

export const useFeaturesStore = create<FeaturesState>((set) => ({
  features: UNKNOWN,

  load: async () => {
    try {
      set({ features: await ipc.featuresGet() });
    } catch {
      set({ features: UNKNOWN });
    }
    // 缓存先上屏,再去平台核对;拉不到就维持缓存值
    void useFeaturesStore.getState().refresh();
  },

  refresh: async () => {
    try {
      set({ features: await ipc.featuresRefresh() });
    } catch {
      // 平台不可达:保持当前值(缓存或全关),不清空
    }
  },
}));

/** 组件里取单个开关的便捷钩子 */
export function useFeature(name: "login" | "sync" | "ai"): boolean {
  return useFeaturesStore((s) => s.features[name]);
}
