// 终端 AI 协助:自然语言 → 命令。生成后必须由用户确认才注入终端,且不自动执行。
import { useEffect, useRef, useState } from "react";
import { AlertTriangle, CornerDownLeft, Loader2, Sparkles } from "lucide-react";
import { Button } from "@/components/ui";
import { useAiStore } from "@/stores/ai";
import { getTerminalContext, useSessionStore } from "@/stores/session";

/** 命中这些模式的命令给出显著风险提示 */
const RISKY = [
  /\brm\s+-[a-zA-Z]*[rf]/, /\bmkfs\b/, /\bdd\s+if=/, /:\s*\(\)\s*\{.*\};\s*:/,
  /\bshutdown\b/, /\breboot\b/, /\bhalt\b/, /\bkill(all)?\s+-9\b/,
  /\bchmod\s+-R\s+777\b/, /\bchown\s+-R\b/, /\bdrop\s+(table|database)\b/i,
  /\btruncate\b/, />\s*\/dev\/[sh]d[a-z]/, /\bgit\s+push\s+.*--force/,
];

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

  const risky = command ? RISKY.some((re) => re.test(command)) : false;

  const insert = async (execute: boolean) => {
    if (!command || !activeId) return;
    // 来自终端白话:先把那行白话清掉,再写入真正的命令
    if (replaceLineInSession) await clearInputLine(replaceLineInSession);
    // execute=false 时只填入不回车,交由用户自己确认后执行
    await runCommand(execute ? command : `${command}{{cursor}}`);
    onClose();
  };

  return (
    <div
      onClick={onClose}
      style={{
        position: "absolute", inset: 0, zIndex: 20,
        background: "var(--surface-overlay)", backdropFilter: "blur(4px)",
        display: "flex", alignItems: "flex-start", justifyContent: "center", paddingTop: "12vh",
      }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        style={{
          width: 560, maxWidth: "92%", background: "var(--surface-raised)",
          border: "1px solid var(--border-default)", borderRadius: 14,
          boxShadow: "var(--shadow-xl)", overflow: "hidden",
        }}
      >
        <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "12px 16px", borderBottom: "1px solid var(--border-subtle)" }}>
          <Sparkles size={16} style={{ color: "var(--accent-text)" }} />
          <input
            ref={inputRef}
            value={request}
            onChange={(e) => setRequest(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") void generate();
              if (e.key === "Escape") onClose();
            }}
            readOnly={fromTerminal && busy}
            placeholder="用一句话描述你想做什么,例如:找出占用 8080 端口的进程"
            style={{ flex: 1, background: "transparent", border: "none", outline: "none", fontFamily: "var(--font-sans)", fontSize: 13.5, color: "var(--text-strong)" }}
          />
          {busy ? (
            <Loader2 size={15} style={{ animation: "vspin 0.9s linear infinite", color: "var(--text-faint)" }} />
          ) : (
            <span className="kbd" style={{ fontSize: 10.5, padding: "2px 6px" }}>↵</span>
          )}
        </div>

        <div style={{ padding: "12px 16px", display: "flex", flexDirection: "column", gap: 10 }}>
          {!command && !error && (
            <div style={{ fontSize: 12, color: "var(--text-faint)", lineHeight: 1.6 }}>
              {fromTerminal
                ? "检测到你输入的是一句话而不是命令,已交给 AI 翻译。采纳后会替换终端里的这一行。"
                : "AI 会结合当前终端最近的输出生成一条命令。生成后需要你确认才会写入终端,不会自动执行。"}
              {provider && (
                <>
                  <br />
                  当前模型:<span style={{ fontFamily: "var(--font-mono)" }}>{provider.name} · {provider.model}</span>
                </>
              )}
            </div>
          )}
          {error && <div style={{ fontSize: 12.5, color: "var(--danger-text)", lineHeight: 1.6 }}>{error}</div>}
          {command && (
            <>
              <div style={{ background: "#0a0a0b", borderRadius: 10, padding: "10px 12px", fontFamily: "var(--font-mono)", fontSize: 12.5, lineHeight: 1.6, color: "#93edfb", whiteSpace: "pre-wrap", wordBreak: "break-all" }}>
                {command}
              </div>
              {risky && (
                <div style={{ display: "flex", alignItems: "flex-start", gap: 8, background: "var(--danger-subtle)", color: "var(--danger-text)", borderRadius: 10, padding: "9px 12px", fontSize: 12, lineHeight: 1.6 }}>
                  <AlertTriangle size={14} style={{ flex: "none", marginTop: 2 }} />
                  这条命令具有破坏性(删除/重启/强制覆盖等),执行前请再确认一遍目标主机与路径。
                </div>
              )}
              <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                <span style={{ fontSize: 11.5, color: "var(--text-faint)", flex: 1 }}>
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
