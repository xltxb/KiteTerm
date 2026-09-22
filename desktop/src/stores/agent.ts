import { create } from "zustand";
import { ipc } from "@/ipc";
import { parseAgentReply } from "@/lib/agentReply";
import { shouldStop } from "@/lib/agentLoop";
import { useAiStore } from "@/stores/ai";
import type { AgentStepResult } from "@/types";

export interface AgentStep {
  id: string;
  intent: string;
  command: string;
  /** AI 自己的声明 */
  aiReadonly: boolean;
  state: "pending" | "running" | "ok" | "failed" | "skipped";
  /** 本地判定为需批准时的原因 */
  reason?: string;
  output?: string;
  exitCode?: number;
  /** AI 对这步输出的解读 */
  analysis?: string;
  /** 第几次尝试(自我修正时 > 1) */
  attempt: number;
}

export interface AgentTask {
  goal: string;
  sessionId: string;
  steps: AgentStep[];
  running: boolean;
  stopped: boolean;
  consecutiveFailures: number;
  /** 任务终态的总结 */
  summary?: string;
  /** 停止原因 */
  endReason?: string;
}

interface AgentState {
  tasks: Record<string, AgentTask>;
  start: (sessionId: string, goal: string) => Promise<void>;
  approve: (sessionId: string, stepId: string) => Promise<void>;
  skip: (sessionId: string, stepId: string) => void;
  stop: (sessionId: string) => void;
}

/** 回灌给 AI 的输出上限。超长取头尾,中间挖掉。
 *  既省 token,也缩小 prompt injection 的注入面。 */
const FEED_LIMIT = 4096;

function clip(out: string): string {
  if (out.length <= FEED_LIMIT) return out;
  const half = FEED_LIMIT / 2;
  const cut = out.length - FEED_LIMIT;
  return out.slice(0, half) + `\n…(已省略 ${cut} 字符)…\n` + out.slice(-half);
}

/** 把终端输出包进带界标的数据块。
 *
 *  界标带一个**每轮随机**的标识。固定字符串会被攻击者预埋 —— 他只要在输出里写一个
 *  伪造的 END 界标,后面接上诱导文字,模型就会以为数据块提前结束、把那段当成指令。
 *  他控制的是上一步的输出,而 nonce 这一轮才生成,预埋时无从知道。
 *
 *  手法与 agent.rs 的退出码哨兵一致,那里的注释写了同样的理由。 */
function asDataBlock(step: AgentStep, nonce: string): string {
  return [
    `命令: ${step.command}`,
    `退出码: ${step.exitCode}`,
    `--- BEGIN TERMINAL OUTPUT ${nonce} ---`,
    clip(step.output ?? ""),
    `--- END TERMINAL OUTPUT ${nonce} ---`,
  ].join("\n");
}

export const useAgentStore = create<AgentState>((set, get) => {
  function getTask(sessionId: string): AgentTask | undefined {
    return get().tasks[sessionId];
  }

  function patch(sessionId: string, p: Partial<AgentTask>) {
    set((s) => {
      const task = s.tasks[sessionId];
      if (!task) return s;
      return { tasks: { ...s.tasks, [sessionId]: { ...task, ...p } } };
    });
  }

  function stepOf(sessionId: string, stepId: string): AgentStep | undefined {
    return getTask(sessionId)?.steps.find((st) => st.id === stepId);
  }

  function patchStep(sessionId: string, stepId: string, p: Partial<AgentStep>) {
    set((s) => {
      const task = s.tasks[sessionId];
      if (!task) return s;
      const steps = task.steps.map((st) => (st.id === stepId ? { ...st, ...p } : st));
      return { tasks: { ...s.tasks, [sessionId]: { ...task, steps } } };
    });
  }

  function patchLastStep(sessionId: string, p: Partial<AgentStep>) {
    const task = getTask(sessionId);
    const last = task?.steps[task.steps.length - 1];
    if (last) patchStep(sessionId, last.id, p);
  }

  function pushStep(sessionId: string, step: AgentStep) {
    set((s) => {
      const task = s.tasks[sessionId];
      if (!task) return s;
      return { tasks: { ...s.tasks, [sessionId]: { ...task, steps: [...task.steps, step] } } };
    });
  }

  function setRunning(sessionId: string, running: boolean) {
    patch(sessionId, { running });
  }

  function bumpFailures(sessionId: string, ok: boolean) {
    const task = getTask(sessionId);
    if (!task) return;
    patch(sessionId, { consecutiveFailures: ok ? 0 : task.consecutiveFailures + 1 });
  }

  function attemptOf(task: AgentTask, command: string): number {
    return task.steps.filter((st) => st.command === command).length + 1;
  }

  function finish(sessionId: string, endReason?: string, summary?: string) {
    patch(sessionId, { running: false, endReason, summary });
  }

  /** 走 stores/ai 的既有通道请求 AI。system prompt 由 Rust 侧按 "agent" mode 决定,
   *  terminalContext 传空串 —— 这个循环自己把上一步输出包成数据块塞进 messages。 */
  async function askAi(goal: string, feed: string): Promise<string> {
    const messages = [{ role: "user" as const, content: feed ? `目标: ${goal}\n\n${feed}` : `目标: ${goal}` }];
    return ipc.aiAsk(messages, "", "agent");
  }

  /** 落一步的结果,返回下一轮要喂给 AI 的内容 */
  function settle(sessionId: string, stepId: string, r: AgentStepResult): string {
    const ok = r.exitCode === 0;
    patchStep(sessionId, stepId, {
      state: ok ? "ok" : "failed",
      output: r.output,
      exitCode: r.exitCode,
    });
    bumpFailures(sessionId, ok); // ok 清零,否则 +1
    const step = stepOf(sessionId, stepId);
    if (!step) return "";
    // 每轮新的界标标识
    const nonce = crypto.randomUUID().slice(0, 8);
    return `本轮数据块标识: ${nonce}\n\n${asDataBlock(step, nonce)}`;
  }

  /** 推进循环,直到需要用户介入或闸门触发。feed 是要喂给 AI 的内容。 */
  async function run(sessionId: string, feed: string) {
    for (;;) {
      const task = getTask(sessionId);
      if (!task) return;

      const stop = shouldStop({
        steps: task.steps.length,
        consecutiveFailures: task.consecutiveFailures,
        stopped: task.stopped,
      });
      if (stop) return finish(sessionId, stop);

      let raw: string;
      try {
        raw = await askAi(task.goal, feed); // 走 stores/ai 的既有通道
      } catch (e) {
        return finish(sessionId, friendly(e));
      }
      const reply = parseAgentReply(raw);
      if ("error" in reply) return finish(sessionId, reply.error);

      // 把这轮的解读挂到上一步的卡片上
      patchLastStep(sessionId, { analysis: reply.analysis });

      if (reply.done) return finish(sessionId, undefined, reply.summary);

      const next = reply.next!;
      const step: AgentStep = {
        id: crypto.randomUUID(),
        intent: next.intent,
        command: next.command,
        aiReadonly: next.readonly,
        state: "running",
        attempt: attemptOf(task, next.command),
      };
      pushStep(sessionId, step);

      let r: AgentStepResult;
      try {
        r = await ipc.agentExec(sessionId, next.command, next.readonly, false);
      } catch (e) {
        // 会话已关闭 / SSH 掉线 / 已中止等:后端会返回 Err,不能让任务永远卡在 running
        patchStep(sessionId, step.id, { state: "failed" });
        return finish(sessionId, friendly(e));
      }

      if (r.verdict === "needs_approval" && !r.autoRan) {
        // 停下等用户点批准。approve() 会续上循环。
        patchStep(sessionId, step.id, { state: "pending", reason: r.reason });
        setRunning(sessionId, false);
        return;
      }

      feed = settle(sessionId, step.id, r);
    }
  }

  /** 包住整个循环的兜底。
   *
   *  run() 是个长循环,里面任何一处未预料的异常都会让任务卡在 running:true ——
   *  UI 一直转圈,用户只能手动停止,而且得不到任何解释。逐个给 await 包 catch 是
   *  打地鼠,这里统一兜住:循环怎么死的不重要,重要的是任务状态必须落到终态。 */
  async function safeRun(sessionId: string, feed: string) {
    try {
      await run(sessionId, feed);
    } catch (e) {
      finish(sessionId, friendly(e));
    }
  }

  return {
    tasks: {},

    start: async (sessionId, goal) => {
      try {
        // 复用 stores/ai 的既有判定:没配置模型就别白跑一轮网络请求
        const hasProvider = !!useAiStore.getState().activeProvider();
        set((s) => ({
          tasks: {
            ...s.tasks,
            [sessionId]: {
              goal,
              sessionId,
              steps: [],
              running: hasProvider,
              stopped: false,
              consecutiveFailures: 0,
              endReason: hasProvider ? undefined : "尚未配置可用的 AI 模型 —— 在右栏「AI 助手」里添加。",
            },
          },
        }));
        if (hasProvider) await safeRun(sessionId, "");
      } catch (e) {
        // action 里任何一步抛异常都不能让任务卡在 running ——
        // settle() 里的 nonce 生成就在 safeRun 的保护范围之外
        finish(sessionId, friendly(e));
      }
    },

    approve: async (sessionId, stepId) => {
      try {
        const step = stepOf(sessionId, stepId);
        // 用户已经点过停止:这一步不再放行,否则「停止」形同虚设 ——
        // 命令仍会真的发到远端执行,只是要等它跑完下一轮才收尾。
        if (!step || step.state !== "pending" || getTask(sessionId)?.stopped) return;
        patchStep(sessionId, stepId, { state: "running" });
        setRunning(sessionId, true);
        let r: AgentStepResult;
        try {
          r = await ipc.agentExec(sessionId, step.command, step.aiReadonly, true);
        } catch (e) {
          // 会话已关闭 / SSH 掉线 / 已中止等:后端会返回 Err,不能让任务永远卡在 running
          patchStep(sessionId, stepId, { state: "failed" });
          return finish(sessionId, friendly(e));
        }
        const feed = settle(sessionId, stepId, r);
        await safeRun(sessionId, feed);
      } catch (e) {
        // action 里任何一步抛异常都不能让任务卡在 running ——
        // settle() 里的 nonce 生成就在 safeRun 的保护范围之外
        finish(sessionId, friendly(e));
      }
    },

    skip: (sessionId, stepId) => {
      try {
        const step = stepOf(sessionId, stepId);
        // 同 approve():停止之后不再受理跳过,任务已经在收尾路上了
        if (!step || step.state !== "pending" || getTask(sessionId)?.stopped) return;
        patchStep(sessionId, stepId, { state: "skipped" });
        setRunning(sessionId, true);
        // 告诉 AI 这步被跳过了,让它换个路子 —— 跳过不计入连续失败
        void safeRun(sessionId, `用户跳过了这一步(${step.command}),请换一个方式。`);
      } catch (e) {
        // action 里任何一步抛异常都不能让任务卡在 running ——
        // settle() 里的 nonce 生成就在 safeRun 的保护范围之外
        finish(sessionId, friendly(e));
      }
    },

    stop: (sessionId) => {
      // 只置标志位:循环下一轮开头的 shouldStop 会接住它。
      // 已经发出去的命令仍在远端跑,这里管不着 —— 界面上如实说明。
      patch(sessionId, { stopped: true, running: false });
    },
  };
});

/** 把 ipc.aiAsk 抛出的错误整理成用户能看懂的文案,与 stores/ai 里的同名逻辑保持一致。 */
function friendly(e: unknown): string {
  const s = String(e);
  if (s.includes("AI_NOT_CONFIGURED")) return "尚未配置可用的 AI 模型 —— 点右上角齿轮添加。";
  return "请求失败:" + s.replace(/^(?:Error:\s*)?[A-Za-z_]+:\s*/, "");
}
