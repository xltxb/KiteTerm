import { create } from "zustand";
import { ipc } from "@/ipc";
import { createAgentEngine, DISABLED_REASON, type AgentTask } from "@/lib/agentEngine";
import { useAiStore } from "@/stores/ai";
import { useFeaturesStore } from "@/stores/features";

export type { AgentStep, AgentTask } from "@/lib/agentEngine";

interface AgentState {
  tasks: Record<string, AgentTask>;
  start: (sessionId: string, goal: string) => Promise<void>;
  approve: (sessionId: string, stepId: string) => Promise<void>;
  skip: (sessionId: string, stepId: string) => void;
  stop: (sessionId: string) => void;
  /** 一步被中断后暂停着,用户选择继续 */
  resume: (sessionId: string) => void;
  /** 停掉所有会话的任务 */
  stopAll: (endReason: string) => void;
}

/** AI 分步执行是否可用:总开关关了子项一律关(与 useAiFeature 同一口径) */
function agentEnabled(): boolean {
  const f = useFeaturesStore.getState().features;
  return f.ai && f.aiAgent;
}

/** 循环本体在 lib/agentEngine.ts,这里只把 zustand、ipc 和功能开关接上去。 */
export const useAgentStore = create<AgentState>((set, get) => {
  const engine = createAgentEngine({
    getTask: (id) => get().tasks[id],
    putTask: (id, task) => set((s) => ({ tasks: { ...s.tasks, [id]: task } })),
    allSessionIds: () => Object.keys(get().tasks),
    // system prompt 由 Rust 侧按 "agent" mode 决定;terminalContext 传空串 ——
    // 循环自己把历史步骤包成数据块放进 messages
    aiAsk: (messages) => ipc.aiAsk(messages, "", "agent"),
    agentExec: (sessionId, command, aiReadonly, approved) =>
      ipc.agentExec(sessionId, command, aiReadonly, approved),
    agentCancel: (sessionId) => ipc.agentCancel(sessionId),
    agentReset: (sessionId) => ipc.agentReset(sessionId),
    hasProvider: () => !!useAiStore.getState().activeProvider(),
    enabled: agentEnabled,
    newId: () => crypto.randomUUID(),
    nonce: () => crypto.randomUUID().slice(0, 8),
  });

  return {
    tasks: {},
    start: engine.start,
    approve: engine.approve,
    skip: engine.skip,
    stop: engine.stop,
    resume: engine.resume,
    stopAll: engine.stopAll,
  };
});

// 管理员在任务进行中关掉开关:面板会整个换成「已关闭」提示、停止按钮随之消失,
// 所以不能等用户来点停止 —— 这里直接把所有任务落到终态。循环本身每轮也会检查开关,
// 这里是让界面立即反映,而不是等正在执行的那一步跑完。
useFeaturesStore.subscribe((s) => {
  if (!(s.features.ai && s.features.aiAgent)) {
    useAgentStore.getState().stopAll(DISABLED_REASON);
  }
});
