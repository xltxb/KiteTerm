/** 执行循环的闸门自检。三道闸任一触发就把控制权交回用户。 */
import { MAX_CONSECUTIVE_FAILURES, MAX_STEPS, shouldStop } from "./agentLoop.ts";

let failed = 0;
function check(cond: boolean, msg: string) {
  if (!cond) { console.error("FAIL: " + msg); failed++; }
}

check(shouldStop({ steps: 1, consecutiveFailures: 0, stopped: false }) === null, "正常推进不停");
check(shouldStop({ steps: 1, consecutiveFailures: 1, stopped: false }) === null,
  "失败 1 次还能自改一次 —— sed 失败改 sudo 正是这一步");
check(shouldStop({ steps: 1, consecutiveFailures: MAX_CONSECUTIVE_FAILURES, stopped: false }) !== null,
  "连续失败到上限要停");
check(shouldStop({ steps: MAX_STEPS, consecutiveFailures: 0, stopped: false }) !== null,
  "步数到上限要停");
check(shouldStop({ steps: 1, consecutiveFailures: 0, stopped: true }) !== null,
  "用户叫停立刻停");
// 用户叫停优先于其它原因,理由文案应体现是用户主动停的
check((shouldStop({ steps: 99, consecutiveFailures: 9, stopped: true }) ?? "").includes("停止"),
  "用户叫停时理由应说明是用户停的");

if (failed > 0) { console.error(`agentLoop 自检未通过:${failed} 项`); process.exit(1); }
console.log("AGENT_LOOP_PASS");
