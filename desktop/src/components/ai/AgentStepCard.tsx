// 分步执行的单张卡片。核心约束:「AI 说的」(intent / analysis)与「机器记的」
// (命令原文 / 退出码 / 输出)必须在视觉上分开 —— AI 读的是终端输出,而输出可被
// 攻击者控制,analysis 本身可能被操纵来诱导用户点批准。用户批准时看的必须是
// 命令原文,不是 AI 对它的描述。见 .agent-said(正文字体 + ✦ 前缀)与
// .agent-cmd/.agent-out(等宽字体、带边框)的样式区分。
import { useEffect, useRef, useState } from "react";
import { Loader2 } from "lucide-react";
import { Badge, Button } from "@/components/ui";
import { isRisky } from "@/lib/riskyCommand";
import type { AgentStep } from "@/stores/agent";

const MAX_LINES = 8;

export function AgentStepCard({
  step,
  stopped,
  onApprove,
  onSkip,
}: {
  step: AgentStep;
  /** 任务已被用户停止 —— pending 态不再提供批准/跳过入口 */
  stopped: boolean;
  onApprove: () => void;
  onSkip: () => void;
}) {
  const [armed, setArmed] = useState(false);   // 破坏性命令的二次确认
  const [open, setOpen] = useState(false);     // 长输出展开
  const [elapsed, setElapsed] = useState(0);   // 执行中已用秒数
  const startRef = useRef<number | null>(null);
  const risky = isRisky(step.command);

  // 执行中每秒刷新一次已用时长;离开 running 态就清零,下次进入重新计时
  useEffect(() => {
    if (step.state !== "running") {
      startRef.current = null;
      setElapsed(0);
      return;
    }
    startRef.current = Date.now();
    setElapsed(0);
    const timer = setInterval(() => {
      setElapsed(Math.floor((Date.now() - (startRef.current ?? Date.now())) / 1000));
    }, 1000);
    return () => clearInterval(timer);
  }, [step.id, step.state]);

  return (
    <div className={`card agent-step is-${step.state}`}>
      {step.attempt > 1 && (
        <Badge color="warning">第 {step.attempt} 次尝试</Badge>
      )}

      {/* 命令原文排在 AI 的描述之前、字号不小于它、底色与卡片拉开 ——
          AI 读的是终端输出,而输出是攻击者可控的,intent 本身可能是被操纵的产物。
          用户批准时眼睛必须落在命令上,排版要主动把注意力导过去,不能靠用户自觉。 */}
      <pre className="agent-cmd">{step.command}</pre>

      {/* AI 说的:正文字体 + ✦ 前缀,颜色降一档表达「这是辅助信息」 */}
      {step.intent && <p className="agent-said">✦ {step.intent}</p>}

      {step.state === "pending" && (
        stopped ? (
          <p className="t-faint fs-sm">任务已停止,这一步不再受理批准或跳过。</p>
        ) : (
          <>
            {step.reason && <p className="t-muted fs-sm">需要批准:{step.reason}</p>}
            {risky && <p className="t-danger fs-sm">这条命令有破坏性,请确认后再执行。</p>}
            <div className="row gap-8">
              <Button
                variant={risky ? "danger" : "primary"}
                onClick={() => {
                  if (risky && !armed) { setArmed(true); return; }
                  onApprove();
                }}
              >
                {risky && armed ? "确认执行?" : "批准执行"}
              </Button>
              <Button variant="ghost" onClick={onSkip}>跳过这步</Button>
            </div>
          </>
        )
      )}

      {step.state === "running" && (
        <div className="row gap-6 t-muted fs-sm">
          <Loader2 size={13} className="spin" />
          执行中…{elapsed > 0 && `${elapsed}s`}
        </div>
      )}

      {(step.state === "ok" || step.state === "failed") && (
        <>
          <Badge color={step.exitCode === 0 ? "success" : "danger"}>
            退出码 {step.exitCode}
          </Badge>
          <pre className="agent-out">
            {open ? step.output : clamp(step.output ?? "", MAX_LINES)}
          </pre>
          {lineCount(step.output ?? "") > MAX_LINES && (
            <Button variant="ghost" onClick={() => setOpen((v) => !v)}>
              {open ? "收起" : "展开"}
            </Button>
          )}
          {step.analysis && <p className="agent-said">✦ {step.analysis}</p>}
        </>
      )}
    </div>
  );
}

function lineCount(s: string) { return s.split("\n").length; }
function clamp(s: string, n: number) { return s.split("\n").slice(0, n).join("\n"); }
