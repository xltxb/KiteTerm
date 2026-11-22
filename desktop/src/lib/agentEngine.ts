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
  /** 本地判定为需批准时的原因;被阻塞时是给用户看的解除办法 */
  reason?: string;
  /** 后端说上一步可能还在终端里运行(BUSY),这一步没能执行。等用户确认终端已空闲后重试 */
  blocked?: "busy";
  output?: string;
  exitCode?: number;
  /** 执行本身出错(会话断开等),没有退出码 —— 卡片显示这个而不是空的退出码 */
  error?: string;
  /** 输出超了上限、前半截被丢掉了 —— 卡片要标出来,别让人(和 AI)以为这就是全部 */
  truncated?: boolean;
  /** 这一步怎么结束的。timeout / interrupted 时退出码只是占位,AI 与卡片都要看这个 */
  endedBy?: AgentStepResult["endedBy"];
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
  /** 一步被中断后暂停在这里,等用户选「继续」或「停止」。值是给用户看的说明 */
  paused?: string;
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
  /** 通知后端:正在执行的这一步作废(命令可能还在跑,后端转入残留状态) */
  agentCancel(sessionId: string): Promise<void>;
  /** 通知后端:用户确认终端已回到命令提示符,清掉等待与残留 */
  agentReset(sessionId: string): Promise<void>;
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
  // 退出码 -1 / 130 本身说不清怎么回事:不写明的话,AI 会把还在跑的命令当成已失败、
  // 马上重试同一条,结果撞上后端的残留状态(#19)
  if (step.endedBy === "timeout") {
    lines.push("结果: 等待超时,命令可能仍在终端里运行,上面的退出码只是占位。不要马上重试同一条命令。");
  } else if (step.endedBy === "interrupted") {
    lines.push("结果: 已被用户中断(Ctrl-C 或停止),上面的退出码只是占位。");
  }
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

/** 去掉 ipc 错误前面的 `Error:` 与 `ERROR_CODE:`,只留给人看的那句 */
function stripCode(e: unknown): string {
  return String(e).replace(/^(?:Error:\s*)?[A-Za-z_]+:\s*/, "");
}

/** 把 ipc 抛出的错误整理成用户能看懂的文案,与 stores/ai 里的同名逻辑保持一致。 */
export function friendly(e: unknown): string {
  const s = String(e);
  if (s.includes("AI_NOT_CONFIGURED")) return "尚未配置可用的 AI 模型 —— 点右上角齿轮添加。";
  return "请求失败:" + stripCode(s);
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
      blocked: undefined,
      state: ok ? "ok" : "failed",
      output: r.output,
      exitCode: r.exitCode,
      truncated: r.truncated,
      endedBy: r.endedBy,
    });
    const t = taskOf(sessionId, runId);
    if (t) patch(sessionId, runId, { consecutiveFailures: ok ? 0 : t.consecutiveFailures + 1 });
  }

  /** 执行一步并落结果。返回 "next" 表示可以继续问 AI;"halt" 表示停下了 ——
   *  等用户批准、等用户确认终端已空闲,或任务已经结束。 */
  async function execute(
    sessionId: string,
    runId: string,
    step: AgentStep,
    approved: boolean,
  ): Promise<"next" | "halt"> {
    // 发命令前最后确认一次:调用方可能刚 await 过别的(approve 里等 agentReset),
    // 用户在这期间点了停止的话,命令不能再发出去(#22)。这一步放回 pending,
    // 卡片会显示「任务已停止」,而不是一直转圈
    const before = taskOf(sessionId, runId);
    if (!before || before.stopped) {
      patchStep(sessionId, runId, step.id, { state: "pending" });
      return "halt";
    }
    let r: AgentStepResult;
    try {
      r = await deps.agentExec(sessionId, step.command, step.aiReadonly, approved);
    } catch (e) {
      if (String(e).includes("AGENT_BUSY")) {
        // 上一步的命令可能还在终端里运行(#18)。不能直接结束任务 —— 停下来,让用户看一眼
        // 终端,确认回到命令提示符后点「终端已空闲」,approve() 会通知后端清掉残留再重试
        const t = taskOf(sessionId, runId);
        if (!t || t.stopped) return "halt";
        patchStep(sessionId, runId, step.id, { state: "pending", blocked: "busy", reason: stripCode(e) });
        patch(sessionId, runId, { running: false, status: "waiting" });
        return "halt";
      }
      // 会话已关闭 / SSH 掉线 / 已中止等:后端会返回 Err,不能让任务永远卡在 running
      patchStep(sessionId, runId, step.id, { state: "failed", error: friendly(e) });
      finish(sessionId, runId, friendly(e));
      return "halt";
    }

    if (r.verdict === "needs_approval" && !r.autoRan) {
      // 用户可能正好在这个窗口点了停止 —— stop() 已经把 status 置成了终态,
      // 这里不能把它拉回 waiting:卡片会重新出现,而 stopped 守卫又让批准/跳过
      // 点了没反应。taskOf 对不上(已开新任务)时同样直接交回。
      const t = taskOf(sessionId, runId);
      if (!t || t.stopped) return "halt";
      // 停下等用户点批准,approve() 会续上循环。status 必须显式置成 waiting ——
      // return 之后再没有代码会来补终态。
      patchStep(sessionId, runId, step.id, { state: "pending", blocked: undefined, reason: r.reason });
      patch(sessionId, runId, { running: false, status: "waiting" });
      return "halt";
    }

    settle(sessionId, runId, step.id, r);

    // 被中断(Ctrl-C)多半是用户想停下来:不自动去问 AI 下一步,先暂停等用户选。
    // 而且批准的步骤被中断后后端处于残留状态,自动推进的下一步必然被阻塞(#24)
    if (r.endedBy === "interrupted") {
      const t = taskOf(sessionId, runId);
      if (!t || t.stopped) return "halt";
      patch(sessionId, runId, {
        running: false,
        status: "waiting",
        paused: "这一步被中断了。要让 AI 接着推进就点「继续」,否则点「停止」。",
      });
      return "halt";
    }
    return "next";
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

      if ((await execute(sessionId, runId, step, false)) === "halt") return;
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
    // 但后端还占着「等这一步」的状态:不通知它作废,下一个任务会一直撞上 BUSY(#18)。
    // 停在等批准 / 被阻塞时后端并没有在等,不用通知。
    if (t.steps.some((st) => st.state === "running")) {
      void deps.agentCancel(sessionId).catch(() => {});
    }
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
      // 被阻塞的步骤:用户确认的是「终端已空闲」,不是批准这条命令 ——
      // 先让后端清掉残留,再按自动执行重试,命令仍要过只读判定
      const wasBlocked = step.blocked === "busy";
      try {
        patchStep(sessionId, runId, stepId, { state: "running", blocked: undefined });
        patch(sessionId, runId, { running: true, status: "running" });
        if (wasBlocked) {
          try {
            await deps.agentReset(sessionId);
          } catch (e) {
            patchStep(sessionId, runId, stepId, { state: "failed", error: friendly(e) });
            return finish(sessionId, runId, friendly(e));
          }
        }
        const cur = taskOf(sessionId, runId)?.steps.find((st) => st.id === stepId);
        if (!cur) return;
        if ((await execute(sessionId, runId, cur, !wasBlocked)) === "next") {
          await safeRun(sessionId, runId);
        }
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

    /** 一步被中断后暂停着,用户选择继续推进 */
    resume(sessionId: string) {
      const task = deps.getTask(sessionId);
      if (!task || !task.paused || task.stopped) return;
      const { runId } = task;
      patch(sessionId, runId, { paused: undefined, running: true, status: "running" });
      void safeRun(sessionId, runId);
    },

    /** 停掉所有会话的任务 —— 管理员关闭功能开关时调用 */
    stopAll(endReason: string) {
      for (const id of deps.allSessionIds()) stopTask(id, endReason);
    },
  };
}
