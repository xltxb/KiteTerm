/** AI 分步执行的循环。
 *
 *  与 UI、ipc、状态库都解耦 —— 这些都通过 EngineDeps 注入,stores/agent.ts 只是
 *  一层薄包装。这样循环里那些和时序相关的毛病(停止竞态、旧循环劫持新任务)能在
 *  node 里直接用假实现逐拍推进来测,见 agentEngine.test.ts。 */
import { parseAgentReply } from "./agentReply.ts";
import { shouldStop } from "./agentLoop.ts";
import type { AgentStepResult } from "../types.ts";

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
  /** 执行本身出错(会话断开等),没有退出码 —— 卡片显示这个而不是空的退出码 */
  error?: string;
  /** 输出超了上限、前半截被丢掉了 —— 卡片要标出来,别让人(和 AI)以为这就是全部 */
  truncated?: boolean;
  /** AI 对这步输出的解读 */
  analysis?: string;
  /** 第几次尝试(自我修正时 > 1) */
  attempt: number;
}

export interface AgentTask {
  /** 这次任务的标识。停止后立刻开新任务时,旧循环还在 await,回来后靠它认出
   *  「任务已经不是我的了」,不能接着往新任务里写东西。 */
  runId: string;
  goal: string;
  sessionId: string;
  steps: AgentStep[];
  running: boolean;
  stopped: boolean;
  consecutiveFailures: number;
  /** 任务的生命周期状态。不要用 running/summary 去推导 ——
   *  停止发生在等待批准时、或 AI 只回 done 不给 summary 时,推导都会得出错误答案。 */
  status: "running" | "waiting" | "ended";
  /** 任务终态的总结 */
  summary?: string;
  /** 停止原因 */
  endReason?: string;
}

export interface ChatMessage {
  role: "user" | "assistant";
  content: string;
}

export interface EngineDeps {
  getTask(sessionId: string): AgentTask | undefined;
  putTask(sessionId: string, task: AgentTask): void;
  allSessionIds(): string[];
  aiAsk(messages: ChatMessage[]): Promise<string>;
  agentExec(sessionId: string, command: string, aiReadonly: boolean, approved: boolean): Promise<AgentStepResult>;
  /** 是否配置了可用的 AI 模型 */
  hasProvider(): boolean;
  /** 管理员是否允许 AI 分步执行。任务进行中也可能被关掉,循环每轮都要看 */
  enabled(): boolean;
  newId(): string;
  /** 数据块界标用的随机串 */
  nonce(): string;
}

/** 最近一步的输出回灌上限。超长取头尾,中间挖掉。既省 token,也缩小注入面。 */
const FEED_LIMIT_LATEST = 4096;
/** 更早步骤的输出回灌上限。AI 要的是「之前发现了什么」,不是全文 —— 历史跟着
 *  步数线性增长,不截短的话 12 步就能把上下文撑爆。 */
const FEED_LIMIT_EARLIER = 1024;

export const DISABLED_REASON = "管理员已关闭 AI 分步执行";

function clip(out: string, limit: number): string {
  if (out.length <= limit) return out;
  const half = limit / 2;
  const cut = out.length - limit;
  return out.slice(0, half) + `\n…(已省略 ${cut} 字符)…\n` + out.slice(-half);
}

/** 一步的历史记录。输出包进带界标的数据块。
 *
 *  界标带一个**每次请求随机**的标识。固定字符串会被攻击者预埋 —— 他只要在输出里写一个
 *  伪造的 END 界标,后面接上诱导文字,模型就会以为数据块提前结束、把那段当成指令。
 *  他控制的是之前各步的输出,而 nonce 这次请求才生成,预埋时无从知道。
 *
 *  手法与 agent.rs 的退出码哨兵一致,那里的注释写了同样的理由。 */
function describeStep(step: AgentStep, n: number, nonce: string, limit: number): string {
  const lines = [`### 第 ${n} 步`];
  if (step.intent) lines.push(`意图: ${step.intent}`);
  lines.push(`命令: ${step.command}`);
  if (step.state === "skipped") {
    lines.push("结果: 用户跳过了这一步,没有执行。请换一个方式。");
    return lines.join("\n");
  }
  lines.push(`退出码: ${step.exitCode}`);
  lines.push(`--- BEGIN TERMINAL OUTPUT ${nonce} ---`);
  // 只在真截断时才加这一行 —— 缓冲溢出后前半截已被丢弃,AI 必须知道
  // 自己看到的不是完整输出,否则会据此做出错误判断。
  if (step.truncated) lines.push("输出已被截断,以下不是完整内容。");
  lines.push(clip(step.output ?? "", limit));
  lines.push(`--- END TERMINAL OUTPUT ${nonce} ---`);
  if (step.analysis) lines.push(`你当时的分析: ${step.analysis}`);
  return lines.join("\n");
}

/** 这一轮发给 AI 的完整内容:目标 + 此前每一步(按执行顺序)。
 *
 *  每轮都带上全部历史,而不是只给上一步 —— 只给上一步时,AI 不知道两步之前查到的
 *  PID 和目标有什么关系,会回头重查,多步任务原地打转。 */
function buildPrompt(task: AgentTask, nonce: string): string {
  const done = task.steps.filter((st) => st.state === "ok" || st.state === "failed" || st.state === "skipped");
  if (done.length === 0) return `目标: ${task.goal}`;
  const blocks = done.map((st, i) =>
    describeStep(st, i + 1, nonce, i === done.length - 1 ? FEED_LIMIT_LATEST : FEED_LIMIT_EARLIER),
  );
  return [`目标: ${task.goal}`, `本轮数据块标识: ${nonce}`, "已执行的步骤(从早到晚):", ...blocks].join("\n\n");
}

/** 把 ipc 抛出的错误整理成用户能看懂的文案,与 stores/ai 里的同名逻辑保持一致。 */
export function friendly(e: unknown): string {
  const s = String(e);
  if (s.includes("AI_NOT_CONFIGURED")) return "尚未配置可用的 AI 模型 —— 点右上角齿轮添加。";
  return "请求失败:" + s.replace(/^(?:Error:\s*)?[A-Za-z_]+:\s*/, "");
}

export function createAgentEngine(deps: EngineDeps) {
  /** 只有 runId 对得上才返回 —— 旧循环拿到的永远是 undefined */
  function taskOf(sessionId: string, runId: string): AgentTask | undefined {
    const t = deps.getTask(sessionId);
    return t && t.runId === runId ? t : undefined;
  }

  function patch(sessionId: string, runId: string, p: Partial<AgentTask>) {
    const t = taskOf(sessionId, runId);
    if (t) deps.putTask(sessionId, { ...t, ...p });
  }

  function patchStep(sessionId: string, runId: string, stepId: string, p: Partial<AgentStep>) {
    const t = taskOf(sessionId, runId);
    if (!t) return;
    const steps = t.steps.map((st) => (st.id === stepId ? { ...st, ...p } : st));
    deps.putTask(sessionId, { ...t, steps });
  }

  function finish(sessionId: string, runId: string, endReason?: string, summary?: string) {
    patch(sessionId, runId, { running: false, status: "ended", endReason, summary });
  }

  /** 落一步的结果 */
  function settle(sessionId: string, runId: string, stepId: string, r: AgentStepResult) {
    const ok = r.exitCode === 0;
    patchStep(sessionId, runId, stepId, {
      state: ok ? "ok" : "failed",
      output: r.output,
      exitCode: r.exitCode,
      truncated: r.truncated,
    });
    const t = taskOf(sessionId, runId);
    if (t) patch(sessionId, runId, { consecutiveFailures: ok ? 0 : t.consecutiveFailures + 1 });
  }

  /** 推进循环,直到需要用户介入或闸门触发。每次 await 回来都要重新确认任务还是自己的。 */
  async function run(sessionId: string, runId: string) {
    for (;;) {
      const task = taskOf(sessionId, runId);
      if (!task) return;

      if (!deps.enabled()) return finish(sessionId, runId, DISABLED_REASON);
      const stop = shouldStop({
        steps: task.steps.length,
        consecutiveFailures: task.consecutiveFailures,
        stopped: task.stopped,
      });
      if (stop) return finish(sessionId, runId, stop);

      let raw: string;
      try {
        raw = await deps.aiAsk([{ role: "user", content: buildPrompt(task, deps.nonce()) }]);
      } catch (e) {
        return finish(sessionId, runId, friendly(e));
      }
      const after = taskOf(sessionId, runId);
      if (!after) return;
      if (after.stopped) return finish(sessionId, runId, "已按你的要求停止");
      if (!deps.enabled()) return finish(sessionId, runId, DISABLED_REASON);

      const reply = parseAgentReply(raw);
      if ("error" in reply) return finish(sessionId, runId, reply.error);

      // 把这轮的解读挂到上一步的卡片上
      const last = after.steps[after.steps.length - 1];
      if (last) patchStep(sessionId, runId, last.id, { analysis: reply.analysis });

      if (reply.done) return finish(sessionId, runId, undefined, reply.summary);

      const next = reply.next!;
      const step: AgentStep = {
        id: deps.newId(),
        intent: next.intent,
        command: next.command,
        aiReadonly: next.readonly,
        state: "running",
        attempt: after.steps.filter((st) => st.command === next.command).length + 1,
      };
      const cur = taskOf(sessionId, runId)!;
      deps.putTask(sessionId, { ...cur, steps: [...cur.steps, step] });

      let r: AgentStepResult;
      try {
        r = await deps.agentExec(sessionId, next.command, next.readonly, false);
      } catch (e) {
        // 会话已关闭 / SSH 掉线 / 已中止等:后端会返回 Err,不能让任务永远卡在 running
        patchStep(sessionId, runId, step.id, { state: "failed", error: friendly(e) });
        return finish(sessionId, runId, friendly(e));
      }

      if (r.verdict === "needs_approval" && !r.autoRan) {
        // 用户可能正好在这个窗口点了停止 —— stop() 已经把 status 置成了终态,
        // 这里不能把它拉回 waiting:卡片会重新出现,而 stopped 守卫又让批准/跳过
        // 点了没反应。taskOf 对不上(已开新任务)时同样直接交回。
        const t = taskOf(sessionId, runId);
        if (!t || t.stopped) return;
        // 停下等用户点批准,approve() 会续上循环。status 必须显式置成 waiting ——
        // return 之后再没有代码会来补终态。
        patchStep(sessionId, runId, step.id, { state: "pending", reason: r.reason });
        patch(sessionId, runId, { running: false, status: "waiting" });
        return;
      }

      settle(sessionId, runId, step.id, r);
    }
  }

  /** 包住整个循环的兜底:循环怎么死的不重要,重要的是任务状态必须落到终态。 */
  async function safeRun(sessionId: string, runId: string) {
    try {
      await run(sessionId, runId);
    } catch (e) {
      finish(sessionId, runId, friendly(e));
    }
  }

  function stopTask(sessionId: string, endReason: string) {
    const t = deps.getTask(sessionId);
    if (!t || t.status === "ended") return;
    // 已经发出去的命令仍在远端跑,这里管不着 —— 界面上如实说明。
    deps.putTask(sessionId, { ...t, stopped: true, running: false, status: "ended", endReason });
  }

  return {
    async start(sessionId: string, goal: string) {
      const runId = deps.newId();
      // 复用既有判定:没配置模型就别白跑一轮网络请求
      const hasProvider = deps.hasProvider();
      deps.putTask(sessionId, {
        runId,
        goal,
        sessionId,
        steps: [],
        running: hasProvider,
        stopped: false,
        consecutiveFailures: 0,
        // 没配置模型这条路直接落终态,不会再进 run() 去调用 finish()
        status: hasProvider ? "running" : "ended",
        endReason: hasProvider ? undefined : "尚未配置可用的 AI 模型 —— 在右栏「AI 助手」里添加。",
      });
      if (hasProvider) await safeRun(sessionId, runId);
    },

    async approve(sessionId: string, stepId: string) {
      const task = deps.getTask(sessionId);
      const step = task?.steps.find((st) => st.id === stepId);
      // 用户已经点过停止:这一步不再放行,否则「停止」形同虚设
      if (!task || !step || step.state !== "pending" || task.stopped) return;
      const { runId } = task;
      try {
        patchStep(sessionId, runId, stepId, { state: "running" });
        patch(sessionId, runId, { running: true, status: "running" });
        let r: AgentStepResult;
        try {
          r = await deps.agentExec(sessionId, step.command, step.aiReadonly, true);
        } catch (e) {
          patchStep(sessionId, runId, stepId, { state: "failed", error: friendly(e) });
          return finish(sessionId, runId, friendly(e));
        }
        settle(sessionId, runId, stepId, r);
        await safeRun(sessionId, runId);
      } catch (e) {
        finish(sessionId, runId, friendly(e));
      }
    },

    skip(sessionId: string, stepId: string) {
      const task = deps.getTask(sessionId);
      const step = task?.steps.find((st) => st.id === stepId);
      // 同 approve():停止之后不再受理跳过
      if (!task || !step || step.state !== "pending" || task.stopped) return;
      const { runId } = task;
      patchStep(sessionId, runId, stepId, { state: "skipped" });
      patch(sessionId, runId, { running: true, status: "running" });
      // 跳过的步骤会出现在下一轮的历史里,AI 据此换路子 —— 跳过不计入连续失败
      void safeRun(sessionId, runId);
    },

    stop(sessionId: string) {
      stopTask(sessionId, "已按你的要求停止");
    },

    /** 停掉所有会话的任务 —— 管理员关闭功能开关时调用 */
    stopAll(endReason: string) {
      for (const id of deps.allSessionIds()) stopTask(id, endReason);
    },
  };
}
