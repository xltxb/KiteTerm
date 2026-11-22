// AI 分步执行(Agent)任务面板:给一个目标,AI 分步给出命令,只读命令自动跑,
// 写操作停下来等用户批准,每步落成一张卡片。任务绑定当前活跃会话,顶部固定
// 显示目标主机,防止用户看错命令实际会在哪台机器上执行。
import { useState } from "react";
import { Copy, Play, Square } from "lucide-react";
import { AgentStepCard } from "./AgentStepCard";
import { Button } from "@/components/ui";
import { useAgentStore } from "@/stores/agent";
import { useAiFeature } from "@/stores/features";
import { useSessionStore } from "@/stores/session";

export function AiAgentPane() {
  const aiAgentOn = useAiFeature("aiAgent");
  const session = useSessionStore((s) => s.sessions.find((x) => x.id === s.activeId));
  const task = useAgentStore((s) => (session ? s.tasks[session.id] : undefined));
  const start = useAgentStore((s) => s.start);
  const approve = useAgentStore((s) => s.approve);
  const skip = useAgentStore((s) => s.skip);
  const stop = useAgentStore((s) => s.stop);
  const resume = useAgentStore((s) => s.resume);
  const [goal, setGoal] = useState("");

  if (!aiAgentOn) {
    return <div className="panel-body t-muted fs-sm">该功能已被管理员关闭</div>;
  }

  // agent 目前只支持 SSH 会话:本地会话在后端会直接收到 AGENT_STEP_ABORTED,
  // 干脆在这里拦住,别让用户点了半天才发现跑不动。
  const isLocal = session?.kind === "local";
  // 「结束」看的是显式的 status 字段,不能用 running/summary 去推导 ——
  // 在 pending 步骤上点停止、或 AI 回 done 却没给 summary,这两种情况下
  // running 和 summary/endReason 都推不出正确答案,只有 status 是可靠的。
  const ended = task?.status === "ended";
  const active = !!task && !ended;

  const hostLabel = session
    ? session.kind === "local"
      ? `${session.user}@localhost`
      : `${session.user}@${session.addr}`
    : null;

  const canStart = !!session && !isLocal && !active && goal.trim().length > 0;

  const doStart = () => {
    if (!canStart || !session) return;
    void start(session.id, goal.trim());
  };

  const copySummary = () => {
    if (!task) return;
    const text = [task.summary, task.endReason].filter(Boolean).join("\n\n");
    if (text) void navigator.clipboard.writeText(text);
  };

  return (
    <div className="panel">
      <div className="panel-head agent-head">
        {hostLabel && <span className="fs-xs t-faint mono">目标主机:{hostLabel}</span>}

        {active ? (
          <>
            {/* 一步被中断后暂停:不自动推进,等用户选继续还是停止 */}
            {task?.paused && <p className="t-muted fs-sm">{task.paused}</p>}
            <div className="row gap-8">
              {task?.paused && (
                <Button variant="primary" onClick={() => session && resume(session.id)}>
                  <Play size={13} />继续
                </Button>
              )}
              <Button variant="secondary" onClick={() => session && stop(session.id)}>
                <Square size={13} />停止
              </Button>
            </div>
            <p className="fs-xs t-faint lh-relaxed">
              停止只会中断后续步骤。已经发出的命令仍在远端运行,需要的话请在终端按 Ctrl-C。
            </p>
          </>
        ) : (
          <>
            <div className="row gap-8">
              <input
                className="input w-full"
                value={goal}
                onChange={(e) => setGoal(e.target.value)}
                onKeyDown={(e) => e.key === "Enter" && doStart()}
                disabled={!session || isLocal}
                placeholder={
                  !session
                    ? "先打开一个会话"
                    : isLocal
                      ? "仅支持 SSH 会话"
                      : "描述这次要 AI 完成的目标,例如:找出并释放被占用的 8080 端口"
                }
              />
              <Button variant="primary" disabled={!canStart} onClick={doStart}>
                <Play size={13} />开始
              </Button>
            </div>
            {isLocal && (
              <p className="fs-xs t-faint">AI 分步执行目前仅支持 SSH 会话,本地终端无法使用。</p>
            )}
          </>
        )}
      </div>

      <div className="panel-scroll vscroll col gap-8 py-8">
        {ended && (task.summary || task.endReason) && (
          <div className="card agent-summary">
            {task.summary && <p className="agent-said">✦ {task.summary}</p>}
            {task.endReason && <p className="fs-xs t-faint">{task.endReason}</p>}
            <Button variant="ghost" onClick={copySummary}>
              <Copy size={13} />复制
            </Button>
          </div>
        )}

        {task?.steps.map((step) => (
          <AgentStepCard
            key={step.id}
            step={step}
            stopped={!!task?.stopped}
            onApprove={() => session && void approve(session.id, step.id)}
            onSkip={() => session && skip(session.id, step.id)}
          />
        ))}

        {!task && (
          <p className="fs-sm t-faint lh-relaxed" style={{ padding: "8px 4px" }}>
            给 AI 一个目标,它会分步给出命令;只读命令自动执行,写操作会停下来等你批准。
          </p>
        )}
      </div>
    </div>
  );
}
