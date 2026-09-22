// 终端 AI 协助:自然语言 → 命令。生成后必须由用户确认才注入终端,且不自动执行。
import { useEffect, useRef, useState } from "react";
import { AlertTriangle, CornerDownLeft, Loader2, Sparkles } from "lucide-react";
import { Button } from "@/components/ui";
import { isRisky } from "@/lib/riskyCommand";
import { useAiStore } from "@/stores/ai";
import { getTerminalContext, useSessionStore } from "@/stores/session";

export function AiCommandBar({
  onClose,
  initialRequest,
  /** 该请求来自终端里直接敲的白话 —— 采纳命令前要先清掉那一行 */
  replaceLineInSession,
}: {
  onClose: () => void;
  initialRequest?: string;
  replaceLineInSession?: string;
}) {
  const [request, setRequest] = useState(initialRequest ?? "");
  const [command, setCommand] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const { toCommand, busy, activeProvider } = useAiStore();
  const activeId = useSessionStore((s) => s.activeId);
  const runCommand = useSessionStore((s) => s.runCommand);
  const clearInputLine = useSessionStore((s) => s.clearInputLine);
  const inputRef = useRef<HTMLInputElement>(null);
  const provider = activeProvider();
  const fromTerminal = !!replaceLineInSession;

  useEffect(() => { inputRef.current?.focus(); }, []);

  const generate = async (text?: string) => {
    const q = (text ?? request).trim();
    if (!q || busy) return;
    setError(null);
    setCommand(null);
    try {
      const cmd = await toCommand(q, activeId ? getTerminalContext(activeId, 40) : "");
      if (!cmd.trim()) {
        setError("模型没有给出可用的命令,换个说法试试。");
        return;
      }
      setCommand(cmd.trim());
    } catch (e) {
      const s = String(e);
      setError(
        s.includes("AI_NOT_CONFIGURED")
          ? "尚未配置可用的 AI 模型 —— 在右栏「AI 助手」里添加。"
          : s.replace(/^(?:Error:\s*)?[A-Za-z_]+:\s*/, ""),
      );
    }
  };

  // 从终端白话进来的:直接开始生成,不用再按一次回车
  useEffect(() => {
    if (initialRequest?.trim()) void generate(initialRequest);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const risky = command ? isRisky(command) : false;

  const insert = async (execute: boolean) => {
    if (!command || !activeId) return;
    // 来自终端白话:先把那行白话清掉,再写入真正的命令
    if (replaceLineInSession) await clearInputLine(replaceLineInSession);
    // execute=false 时只填入不回车,交由用户自己确认后执行
    await runCommand(execute ? command : `${command}{{cursor}}`);
    onClose();
  };

  return (
    <div className="sheet-overlay" onClick={onClose}>
      <div className="sheet" onClick={(e) => e.stopPropagation()}>
        <div className="sheet-head">
          <Sparkles size={16} className="t-accent" />
          <input
            ref={inputRef}
            className="sheet-input"
            value={request}
            onChange={(e) => setRequest(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void generate();
              if (e.key === "Escape") onClose();
            }}
            readOnly={fromTerminal && busy}
            placeholder="用一句话描述你想做什么,例如:找出占用 8080 端口的进程"
          />
          {busy ? (
            <Loader2 size={15} className="t-faint spin" />
          ) : (
            <span className="kbd sm">↵</span>
          )}
        </div>

        <div className="sheet-body">
          {!command && !error && (
            <div className="fs-sm t-faint lh-relaxed">
              {fromTerminal
                ? "检测到你输入的是一句话而不是命令,已交给 AI 翻译。采纳后会替换终端里的这一行。"
                : "AI 会结合当前终端最近的输出生成一条命令。生成后需要你确认才会写入终端,不会自动执行。"}
              {provider && (
                <>
                  <br />
                  当前模型:<span className="mono">{provider.name} · {provider.model}</span>
                </>
              )}
            </div>
          )}
          {error && <div className="fs-sm t-danger lh-relaxed">{error}</div>}
          {command && (
            <>
              <div className="code-block fs-sm" style={{ wordBreak: "break-all" }}>
                {command}
              </div>
              {risky && (
                <div className="alert alert-danger">
                  <AlertTriangle size={14} className="fnone" style={{ marginTop: 2 }} />
                  这条命令具有破坏性(删除/重启/强制覆盖等),执行前请再确认一遍目标主机与路径。
                </div>
              )}
              <div className="row gap-8">
                <span className="fs-xs t-faint grow">
                  {activeId ? "写入当前会话" : "没有活跃会话,请先连接主机"}
                </span>
                <Button variant="ghost" onClick={() => { void navigator.clipboard.writeText(command); onClose(); }}>
                  复制
                </Button>
                <Button variant="secondary" disabled={!activeId} onClick={() => insert(false)}>
                  填入终端
                </Button>
                <Button variant="primary" disabled={!activeId} onClick={() => insert(true)}>
                  <CornerDownLeft size={13} />直接运行
                </Button>
              </div>
            </>
          )}
        </div>
      </div>
    </div>
  );
}
