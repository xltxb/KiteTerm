import type { SessionInfo } from "@/types";

type SessionLike = Pick<SessionInfo, "id" | "kind" | "state">;

/**
 * 输入镜像开启时,一次按键要下发到的会话集合:
 * 来源会话本身 + 所有已连接的 SSH 会话(本地终端不参与镜像,断线中的会话跳过)。
 * 关闭时只回来源会话。结果去重,来源会话永远排第一。
 */
export function mirrorTargets(originId: string, sessions: SessionLike[], on: boolean): string[] {
  if (!on) return [originId];
  const ids = sessions
    .filter((s) => s.kind === "ssh" && s.state === "connected" && s.id !== originId)
    .map((s) => s.id);
  return [originId, ...ids];
}
