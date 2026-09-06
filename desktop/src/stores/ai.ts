import { create } from "zustand";
import { ipc } from "@/ipc";
import type { AiProvider, AiSettings } from "@/types";

interface AiMessage {
  role: "user" | "assistant";
  text: string;
}

interface AiState {
  settings: AiSettings;
  loaded: boolean;
  messages: AiMessage[];
  busy: boolean;
  load: () => Promise<void>;
  activeProvider: () => AiProvider | null;
  saveProvider: (p: Omit<AiProvider, "hasKey">, apiKey?: string) => Promise<void>;
  deleteProvider: (id: string) => Promise<void>;
  setActive: (id: string) => Promise<void>;
  /** 侧栏对话:追加用户消息 → 请求 → 追加回复 */
  send: (text: string, terminalContext: string) => Promise<void>;
  /** 终端协助:把一段输出交给 AI 解释,结果进入对话流 */
  explain: (terminalContext: string) => Promise<void>;
  /** 长输出摘要:把一大段终端输出交给 AI 结构化总结,结果进入对话流 */
  summarize: (terminalContext: string) => Promise<void>;
  /** 自然语言 → 单条命令(不进对话流,由调用方确认后注入终端) */
  toCommand: (request: string, terminalContext: string) => Promise<string>;
  clear: () => void;
}

export const useAiStore = create<AiState>((set, get) => ({
  settings: { providers: [], activeId: null },
  loaded: false,
  messages: [],
  busy: false,

  load: async () => {
    set({ settings: await ipc.aiGetSettings(), loaded: true });
  },

  activeProvider: () => {
    const { providers, activeId } = get().settings;
    return providers.find((p) => p.id === activeId) ?? null;
  },

  saveProvider: async (p, apiKey) => {
    set({ settings: await ipc.aiSaveProvider(p, apiKey) });
  },

  deleteProvider: async (id) => {
    set({ settings: await ipc.aiDeleteProvider(id) });
  },

  setActive: async (id) => {
    set({ settings: await ipc.aiSetActive(id) });
  },

  send: async (text, terminalContext) => {
    const next = [...get().messages, { role: "user" as const, text }];
    set({ messages: next, busy: true });
    try {
      const reply = await ipc.aiAsk(
        next.map((m) => ({ role: m.role, content: m.text })),
        terminalContext,
        "chat",
      );
      set({ messages: [...next, { role: "assistant", text: reply }] });
    } catch (e) {
      set({ messages: [...next, { role: "assistant", text: friendly(e) }] });
    } finally {
      set({ busy: false });
    }
  },

  explain: async (terminalContext) => {
    const ask = "解释一下上面这段终端输出。";
    const next = [...get().messages, { role: "user" as const, text: ask }];
    set({ messages: next, busy: true });
    try {
      const reply = await ipc.aiAsk([{ role: "user", content: ask }], terminalContext, "explain");
      set({ messages: [...next, { role: "assistant", text: reply }] });
    } catch (e) {
      set({ messages: [...next, { role: "assistant", text: friendly(e) }] });
    } finally {
      set({ busy: false });
    }
  },

  summarize: async (terminalContext) => {
    const ask = "总结一下上面这段终端输出。";
    const next = [...get().messages, { role: "user" as const, text: ask }];
    set({ messages: next, busy: true });
    try {
      const reply = await ipc.aiAsk([{ role: "user", content: ask }], terminalContext, "summary");
      set({ messages: [...next, { role: "assistant", text: reply }] });
    } catch (e) {
      set({ messages: [...next, { role: "assistant", text: friendly(e) }] });
    } finally {
      set({ busy: false });
    }
  },

  toCommand: async (request, terminalContext) => {
    set({ busy: true });
    try {
      return await ipc.aiAsk([{ role: "user", content: request }], terminalContext, "command");
    } finally {
      set({ busy: false });
    }
  },

  clear: () => set({ messages: [] }),
}));

function friendly(e: unknown): string {
  const s = String(e);
  if (s.includes("AI_NOT_CONFIGURED")) return "尚未配置可用的 AI 模型 —— 点右上角齿轮添加。";
  return "请求失败:" + s.replace(/^(?:Error:\s*)?[A-Za-z_]+:\s*/, "");
}
