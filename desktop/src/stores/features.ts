import { create } from "zustand";
import { ipc } from "@/ipc";
import type { Features } from "@/types";

/** 从未拉到过时的兜底:按未启用门禁处理,不限制任何功能。
 *  与 Rust 侧 Features::unknown 保持一致。 */
const UNKNOWN: Features = {
  registration: "open",
  enforced: false,
  login: true,
  sync: true,
  ai: true,
  keySync: true,
  recordingSync: true,
  aiCommand: true,
  aiDiagnose: true,
  aiSummary: true,
  aiAgent: true,
  emailVerify: false,
  totpRequired: false,
  multiDevice: true,
  passwordMinLength: 8,
  passwordRule: "",
  known: false,
};

/** 原型约定:变更在客户端下一次心跳(≤60 s)内生效 */
const POLL_MS = 60_000;
let pollTimer: ReturnType<typeof setInterval> | null = null;

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
    // 全局只起一个轮询,跟应用同生命周期,不清理
    if (!pollTimer) pollTimer = setInterval(() => void useFeaturesStore.getState().refresh(), POLL_MS);
  },

  refresh: async () => {
    try {
      set({ features: await ipc.featuresRefresh() });
    } catch {
      // 平台不可达:保持当前值(缓存或全关),不清空
    }
  },
}));

export type FeatureName =
  | "login"
  | "sync"
  | "ai"
  | "keySync"
  | "recordingSync"
  | "aiCommand"
  | "aiDiagnose"
  | "aiSummary"
  | "aiAgent"
  | "emailVerify"
  | "totpRequired"
  | "multiDevice";

/** 组件里取单个开关的便捷钩子 */
export function useFeature(name: FeatureName): boolean {
  return useFeaturesStore((s) => s.features[name]);
}

/** AI 子功能:总开关关了子项一律关 */
export function useAiFeature(name: "aiCommand" | "aiDiagnose" | "aiSummary" | "aiAgent"): boolean {
  return useFeaturesStore((s) => s.features.ai && s.features[name]);
}
