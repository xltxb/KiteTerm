/**
 * 输入镜像目标集合用例：node --experimental-strip-types src/lib/broadcast.test.ts
 */
import { mirrorTargets } from "./broadcast.ts";

let failed = 0;
function expect(got: string[], want: string[], why: string) {
  if (JSON.stringify(got) !== JSON.stringify(want)) {
    failed++;
    console.error(`  ✗ ${JSON.stringify(got)} 应为 ${JSON.stringify(want)}（${why}）`);
  }
}

const ssh = (id: string, state = "connected") => ({ id, kind: "ssh" as const, state: state as "connected" });
const local = (id: string) => ({ id, kind: "local" as const, state: "connected" as const });

const sessions = [ssh("a"), ssh("b"), local("l"), ssh("c", "connecting")];

expect(mirrorTargets("a", sessions, false), ["a"], "关闭时只回来源");
expect(mirrorTargets("a", sessions, true), ["a", "b"], "开启时来源在首位,其余已连接 SSH 跟随,不重复");
expect(mirrorTargets("b", sessions, true), ["b", "a"], "来源不是第一个会话时仍排首位");
expect(mirrorTargets("l", sessions, true), ["l", "a", "b"], "从本地终端发起也镜像到 SSH 会话,本地自身保留");
expect(mirrorTargets("a", [ssh("a"), ssh("c", "connecting"), ssh("d", "disconnected")], true), ["a"], "未连接状态的会话跳过");
expect(mirrorTargets("a", [], true), ["a"], "没有其他会话时只回来源");

if (failed) { console.error(`broadcast: ${failed} 个用例失败`); process.exit(1); }
console.log("broadcast: 全部通过");
