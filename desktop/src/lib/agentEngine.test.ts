/** AI 分步执行循环的自检。ipc、AI、状态存取都换成可控的假实现,逐拍推进。 */
import { createAgentEngine, type AgentTask, type EngineDeps } from "./agentEngine.ts";
import type { AgentStepResult } from "../types.ts";

let failed = 0;
function check(cond: boolean, msg: string) {
  if (!cond) { console.error("FAIL: " + msg); failed++; }
}

interface Deferred<T> { promise: Promise<T>; resolve: (v: T) => void; reject: (e: unknown) => void }
function deferred<T>(): Deferred<T> {
  let resolve!: (v: T) => void, reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}
const tick = async () => { for (let i = 0; i < 10; i++) await new Promise((r) => setTimeout(r, 0)); };

function env(opts: { holdReset?: boolean } = {}) {
  const tasks: Record<string, AgentTask> = {};
  const asks: { content: string; d: Deferred<string> }[] = [];
  const execs: { sessionId: string; stepId: string; command: string; approved: boolean; d: Deferred<AgentStepResult> }[] = [];
  const cancels: { sessionId: string; stepId: string }[] = [];
  const resets: string[] = [];
  /** holdReset 时 agentReset 挂起,直到测试调用 releaseReset() */
  const heldResets: (() => void)[] = [];
  let id = 0;
  const state = { enabled: true };
  const deps: EngineDeps = {
    getTask: (s) => tasks[s],
    putTask: (s, t) => { tasks[s] = t; },
    allSessionIds: () => Object.keys(tasks),
    aiAsk: (messages) => {
      const d = deferred<string>();
      asks.push({ content: messages.map((m) => m.content).join("\n"), d });
      return d.promise;
    },
    agentExec: (sessionId, stepId, command, _ro, approved) => {
      const d = deferred<AgentStepResult>();
      execs.push({ sessionId, stepId, command, approved, d });
      return d.promise;
    },
    agentCancel: async (sessionId, stepId) => { cancels.push({ sessionId, stepId }); },
    agentReset: (sessionId) => {
      resets.push(sessionId);
      if (!opts.holdReset) return Promise.resolve();
      return new Promise<void>((res) => heldResets.push(res));
    },
    hasProvider: () => true,
    enabled: () => state.enabled,
    newId: () => `id${++id}`,
    nonce: () => `nonce${++id}`,
  };
  const releaseReset = () => heldResets.shift()?.();
  return { tasks, asks, execs, cancels, resets, releaseReset, state, engine: createAgentEngine(deps) };
}

const next = (command: string, readonly = true) =>
  JSON.stringify({ analysis: `分析:${command}`, next: { intent: "意图", command, readonly }, done: false });
const ran = (output: string, exitCode = 0, endedBy: AgentStepResult["endedBy"] = "exit"): AgentStepResult =>
  ({ autoRan: true, verdict: "readonly", reason: "", output, exitCode, truncated: false, endedBy });
const needsApproval: AgentStepResult =
  { autoRan: false, verdict: "needs_approval", reason: "不在只读白名单内", output: "", exitCode: 0, truncated: false, endedBy: "exit" };

// ---- 基本流程 ----
{
  const e = env();
  void e.engine.start("s", "看看磁盘");
  await tick();
  check(e.asks.length === 1, "开始后先问 AI");
  e.asks[0].d.resolve(next("df -h"));
  await tick();
  check(e.execs.length === 1 && e.execs[0].command === "df -h", "AI 给的命令被执行");
  e.execs[0].d.resolve(ran("/dev/sda1 50%"));
  await tick();
  e.asks[1].d.resolve(JSON.stringify({ analysis: "够用", done: true, summary: "磁盘还剩一半" }));
  await tick();
  check(e.tasks.s.status === "ended" && e.tasks.s.summary === "磁盘还剩一半", "AI 回 done 后任务结束");
}

// ---- 需批准:停下等用户,批准后以 approved=true 执行并继续 ----
{
  const e = env();
  void e.engine.start("s", "重启 nginx");
  await tick();
  e.asks[0].d.resolve(next("systemctl restart nginx", false));
  await tick();
  e.execs[0].d.resolve(needsApproval);
  await tick();
  check(e.tasks.s.status === "waiting", "需批准时任务进入 waiting");
  const step = e.tasks.s.steps[0];
  check(step.state === "pending" && step.reason === "不在只读白名单内", "卡片带上原因");
  void e.engine.approve("s", step.id);
  await tick();
  check(e.execs.length === 2 && e.execs[1].approved, "批准后以 approved=true 执行");
  e.execs[1].d.resolve(ran(""));
  await tick();
  check(e.asks.length === 2, "批准的步骤跑完后继续问 AI");
}

// ---- #4:停止后马上开新任务,旧循环不能劫持新任务 ----
{
  const e = env();
  void e.engine.start("s", "旧目标");
  await tick();
  e.asks[0].d.resolve(next("find / -name x"));
  await tick();
  check(e.execs.length === 1, "旧任务的命令在执行");

  e.engine.stop("s");
  void e.engine.start("s", "新目标");
  await tick();
  check(e.asks.length === 2 && e.asks[1].content.includes("新目标"), "新任务开始问 AI");
  e.asks[1].d.resolve(next("pwd"));
  await tick();
  check(e.execs.length === 2, "新任务的命令在执行");

  // 旧任务那一步现在才返回
  e.execs[0].d.resolve(ran("old output"));
  await tick();
  check(e.asks.length === 2, "旧循环不能再替新任务问 AI");
  check(e.tasks.s.goal === "新目标", "任务还是新任务");
  check(e.tasks.s.steps.length === 1 && e.tasks.s.steps[0].command === "pwd", "旧循环不能往新任务里写步骤");
  check(e.tasks.s.status === "running", "新任务不能被旧循环弄成结束");

  e.execs[1].d.resolve(ran("/root"));
  await tick();
  check(e.asks.length === 3, "新任务照常推进");
}

// ---- #6:AI 每轮要看到此前所有步骤 ----
{
  const e = env();
  void e.engine.start("s", "释放 8080 端口");
  await tick();
  e.asks[0].d.resolve(next("ss -tlnp"));
  await tick();
  e.execs[0].d.resolve(ran("LISTEN 0.0.0.0:8080 pid=1234"));
  await tick();
  e.asks[1].d.resolve(next("ps -p 1234 -o cmd"));
  await tick();
  e.execs[1].d.resolve(ran("java -jar old.jar"));
  await tick();
  const c = e.asks[2].content;
  check(c.includes("释放 8080 端口"), "带目标");
  check(c.includes("pid=1234") && c.includes("java -jar old.jar"), "两步的输出都在");
  check(c.indexOf("ss -tlnp") < c.indexOf("ps -p 1234"), "按执行顺序排列");
  // 第 2 轮的回复(next = ps …)里的 analysis 是对第 1 步 ss 输出的解读,挂在第 1 步上
  check(c.includes("你当时的分析: 分析:ps -p 1234 -o cmd"), "带上 AI 当时对每步的分析");
  // 只声明一个本轮标识,所有数据块都用它
  const m = c.match(/本轮数据块标识: (\S+)/g) ?? [];
  check(m.length === 1, "只声明一次本轮标识");
  const nonce = m[0]?.split(": ")[1] ?? "";
  check(c.split(`--- BEGIN TERMINAL OUTPUT ${nonce} ---`).length - 1 === 2, "每步一个数据块,都用本轮标识");

  // 跳过的步骤也要出现在历史里
  e.asks[2].d.resolve(next("kill 1234", false));
  await tick();
  e.execs[2].d.resolve(needsApproval);
  await tick();
  e.engine.skip("s", e.tasks.s.steps[2].id);
  await tick();
  const c2 = e.asks[3].content;
  check(c2.includes("kill 1234") && c2.includes("跳过"), "跳过的步骤在历史里标明");
  check(c2.includes("pid=1234"), "跳过之后之前的输出还在");
}

// ---- #6:历史不能无限膨胀,早先步骤的输出要截得更短 ----
{
  const e = env();
  void e.engine.start("s", "g");
  await tick();
  for (let i = 0; i < 5; i++) {
    e.asks[i].d.resolve(next(`cat f${i}`));
    await tick();
    e.execs[i].d.resolve(ran("x".repeat(20000)));
    await tick();
  }
  const len = e.asks[5].content.length;
  check(len < 20000, `5 步各 2 万字的输出,喂给 AI 的总长要有上限(实际 ${len})`);
}

// ---- #7:管理员关掉开关后,循环不能再继续 ----
{
  const e = env();
  void e.engine.start("s", "g");
  await tick();
  e.asks[0].d.resolve(next("ls"));
  await tick();
  e.state.enabled = false;
  e.execs[0].d.resolve(ran("a"));
  await tick();
  check(e.asks.length === 1, "关掉之后不再问 AI");
  check(e.tasks.s.status === "ended", "任务落到终态");
  check((e.tasks.s.endReason ?? "").includes("关闭"), "说明是管理员关闭的");
}

// ---- #7:stopAll 停掉所有会话的任务(开关关闭时调用) ----
{
  const e = env();
  void e.engine.start("a", "g");
  void e.engine.start("b", "g");
  await tick();
  e.engine.stopAll("管理员已关闭 AI 分步执行");
  check(e.tasks.a.status === "ended" && e.tasks.b.status === "ended", "所有任务都停下");
  check(e.tasks.a.endReason === "管理员已关闭 AI 分步执行", "停止原因如实记录");
}

// ---- #9:执行出错的步骤要记下原因,卡片据此显示,而不是空退出码 ----
{
  const e = env();
  void e.engine.start("s", "g");
  await tick();
  e.asks[0].d.resolve(next("ls"));
  await tick();
  e.execs[0].d.reject("AGENT_STEP_ABORTED: 这一步没有结果(会话可能已关闭)");
  await tick();
  const st = e.tasks.s.steps[0];
  check(st.state === "failed", "步骤标为失败");
  check(st.exitCode === undefined && (st.error ?? "").includes("会话可能已关闭"), "记下出错原因");
}

// ---- #18:后端说上一步可能还在运行时,这一步停下等用户确认终端已空闲 ----
const BUSY_STALE = "AGENT_BUSY_STALE: 上一步的命令可能还在终端里运行。确认终端已回到命令提示符后,点「终端已空闲」继续。";
{
  const e = env();
  void e.engine.start("s", "g");
  await tick();
  e.asks[0].d.resolve(next("df -h"));
  await tick();
  e.execs[0].d.reject(BUSY_STALE);
  await tick();
  const st = e.tasks.s.steps[0];
  check(e.tasks.s.status === "waiting", "被阻塞时任务进入 waiting,而不是直接结束");
  check(st.state === "pending" && st.blocked === "busy", "这一步标为被阻塞");
  check((st.reason ?? "").includes("终端已空闲") && !(st.reason ?? "").includes("AGENT_BUSY"),
    "原因给人看:写明怎么解除,不带错误码");

  // 用户确认终端已空闲:先通知后端清掉残留,再按自动执行重试(approved=false)
  void e.engine.approve("s", st.id);
  await tick();
  check(e.resets.length === 1 && e.resets[0] === "s", "先通知后端清掉残留");
  check(e.execs.length === 2 && !e.execs[1].approved, "重试时不算用户批准,仍要过只读判定");
  e.execs[1].d.resolve(ran("/dev/sda1 50%"));
  await tick();
  check(e.tasks.s.steps[0].state === "ok" && !e.tasks.s.steps[0].blocked, "重试成功后解除阻塞标记");
  check(e.asks.length === 2, "照常推进到下一轮");
}

// ---- #18:被阻塞的步骤重试后如果需要批准,照常停下等批准 ----
{
  const e = env();
  void e.engine.start("s", "g");
  await tick();
  e.asks[0].d.resolve(next("systemctl restart nginx", false));
  await tick();
  e.execs[0].d.reject(BUSY_STALE);
  await tick();
  void e.engine.approve("s", e.tasks.s.steps[0].id);
  await tick();
  e.execs[1].d.resolve(needsApproval);
  await tick();
  const st = e.tasks.s.steps[0];
  check(st.state === "pending" && !st.blocked && st.reason === "不在只读白名单内", "转为普通的待批准");
  void e.engine.approve("s", st.id);
  await tick();
  check(e.execs.length === 3 && e.execs[2].approved, "这次才是用户批准执行");
  check(e.resets.length === 1, "普通批准不再通知后端清残留");
}

// ---- #18:停止时若有一步正在执行,要通知后端作废这一步 ----
{
  const e = env();
  void e.engine.start("s", "g");
  await tick();
  e.asks[0].d.resolve(next("sudo -i", false));
  await tick();
  e.execs[0].d.resolve(needsApproval);
  await tick();
  e.engine.stop("s");
  check(e.cancels.length === 0, "停在等批准时后端没有在等,不用通知");

  const e2 = env();
  void e2.engine.start("s", "g");
  await tick();
  e2.asks[0].d.resolve(next("df -h"));
  await tick();
  e2.engine.stop("s");
  check(e2.cancels.length === 1 && e2.cancels[0].sessionId === "s", "有一步正在执行时通知后端作废");
  // 作废请求指明是哪一步:晚到时后端可能已在等新任务的步骤,只认这个 id(#23)
  check(e2.cancels[0].stepId === e2.tasks.s.steps[0].id, "作废的正是正在执行的那一步");
  check(e2.execs[0].stepId === e2.tasks.s.steps[0].id, "执行时也带上同一个步骤 id");

  const e3 = env();
  void e3.engine.start("a", "g");
  await tick();
  e3.asks[0].d.resolve(next("df -h"));
  await tick();
  e3.engine.stopAll("管理员已关闭 AI 分步执行");
  check(e3.cancels.length === 1, "stopAll 同样通知");
}

// ---- #19:超时 / 中断的步骤要在历史里写明,不能只给一个 -1 / 130 ----
/** 跑一步,返回下一轮发给 AI 的内容与这一步 */
async function oneStep(result: AgentStepResult) {
  const e = env();
  void e.engine.start("s", "g");
  await tick();
  e.asks[0].d.resolve(next("du -sh /var"));
  await tick();
  e.execs[0].d.resolve(result);
  await tick();
  // 被中断的步骤会先暂停(#24),点「继续」才会把它带进下一轮发给 AI
  if (e.tasks.s.paused) {
    e.engine.resume("s");
    await tick();
  }
  return { content: e.asks[1]?.content ?? "", step: e.tasks.s.steps[0] };
}
{
  const t = await oneStep(ran("12G\t/var/lib", -1, "timeout"));
  check(t.content.includes("等待超时") && t.content.includes("可能仍在"), "超时的步骤写明命令可能仍在运行");
  check(t.step.endedBy === "timeout", "步骤上记下结束方式,卡片据此显示");

  const i = await oneStep(ran("", 130, "interrupted"));
  check(i.content.includes("中断"), "被中断的步骤写明是中断");
  check(i.step.endedBy === "interrupted", "记下被中断");

  const ok = await oneStep(ran("12G\t/var", 0));
  check(!ok.content.includes("超时") && !ok.content.includes("中断"), "正常结束的步骤不加多余说明");
}

// ---- #24:一步被中断(Ctrl-C)后不自动推进,停下等用户选「继续」或「停止」 ----
{
  const e = env();
  void e.engine.start("s", "g");
  await tick();
  e.asks[0].d.resolve(next("sudo apt upgrade", false));
  await tick();
  e.execs[0].d.resolve(needsApproval);
  await tick();
  void e.engine.approve("s", e.tasks.s.steps[0].id);
  await tick();
  e.execs[1].d.resolve(ran("Do you want to continue? [Y/n] ^C", 130, "interrupted"));
  await tick();
  check(e.asks.length === 1, "被中断后不自动去问 AI");
  check(e.tasks.s.status === "waiting" && !!e.tasks.s.paused, "任务暂停,等用户选择");
  check(e.tasks.s.steps[0].state === "failed" && e.tasks.s.steps[0].endedBy === "interrupted", "这一步照实落下中断结果");

  e.engine.resume("s");
  await tick();
  check(e.asks.length === 2 && !e.tasks.s.paused && e.tasks.s.status === "running", "点继续后照常推进");
  check(e.asks[1].content.includes("中断"), "AI 看得到这一步被中断了");
}
{
  const e = env();
  void e.engine.start("s", "g");
  await tick();
  e.asks[0].d.resolve(next("df -h"));
  await tick();
  e.execs[0].d.resolve(ran("", 130, "interrupted"));
  await tick();
  e.engine.stop("s");
  check(e.tasks.s.status === "ended", "暂停时也能直接停止");
  e.engine.resume("s");
  await tick();
  check(e.asks.length === 1, "停止之后点继续不再推进");
}

// ---- #22:确认终端已空闲后、后端还没回话时点了停止,命令不能再执行 ----
{
  const e = env({ holdReset: true });
  void e.engine.start("s", "g");
  await tick();
  e.asks[0].d.resolve(next("df -h"));
  await tick();
  e.execs[0].d.reject(BUSY_STALE);
  await tick();
  void e.engine.approve("s", e.tasks.s.steps[0].id); // 点「终端已空闲,继续」
  await tick();
  check(e.resets.length === 1, "已通知后端清残留,正在等它回话");
  e.engine.stop("s"); // 反悔
  e.releaseReset();
  await tick();
  check(e.execs.length === 1, "停止之后不能再把命令发出去");
  check(e.tasks.s.status === "ended", "任务保持结束");
  check(e.tasks.s.steps[0].state !== "running", "这一步不能一直显示执行中");
}

if (failed > 0) {
  console.error(`agentEngine: ${failed} 项失败`);
  process.exit(1);
}
console.log("agentEngine: 全部通过");
