// AI 助手面板 — 读取当前会话终端输出作为上下文,经后端调用 Claude API
import { useEffect, useRef, useState } from "react";
import { ArrowUp, Loader2, Settings2 } from "lucide-react";
import { Button, TextInput } from "@/components/ui";
import { ipc } from "@/ipc";
import { getTerminalContext, useSessionStore } from "@/stores/session";

interface Msg {
  role: "user" | "assistant";
  text: string;
}

/** 把回复按 ``` 代码块切段渲染,代码块附带 运行/复制 按钮 */
function Reply({ text }: { text: string }) {
  const runCommand = useSessionStore((s) => s.runCommand);
  const activeId = useSessionStore((s) => s.activeId);
  const [copied, setCopied] = useState<number | null>(null);
  const parts = text.split(/```[a-zA-Z]*\n?/);
  return (
    <div style={{ alignSelf: "flex-start", maxWidth: "94%", display: "flex", flexDirection: "column", gap: 8 }}>
      {parts.map((part, i) => {
        const trimmed = part.replace(/\n+$/, "");
        if (!trimmed) return null;
        if (i % 2 === 1) {
          // 代码块
          return (
            <div key={i} style={{ display: "flex", flexDirection: "column", gap: 6 }}>
              <div style={{ background: "#0a0a0b", borderRadius: 10, padding: "9px 12px", fontFamily: "var(--font-mono)", fontSize: 11.5, lineHeight: 1.6, color: "#93edfb", whiteSpace: "pre-wrap", overflowX: "auto" }}>
                {trimmed}
              </div>
              <div style={{ display: "flex", gap: 6 }}>
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={!activeId}
                  onClick={() => void runCommand(trimmed.trim())}
                  style={{ height: 28 }}
                >
                  在终端运行
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  style={{ height: 28 }}
                  onClick={async () => {
                    await navigator.clipboard.writeText(trimmed.trim());
                    setCopied(i);
                    setTimeout(() => setCopied(null), 1500);
                  }}
                >
                  {copied === i ? "已复制 ✓" : "复制"}
                </Button>
              </div>
            </div>
          );
        }
        return (
          <div key={i} style={{ background: "var(--surface-sunken)", border: "1px solid var(--border-subtle)", borderRadius: "12px 12px 12px 4px", padding: "9px 12px", fontSize: 12.5, lineHeight: 1.6, color: "var(--text-body)", whiteSpace: "pre-wrap" }}>
            {trimmed}
          </div>
        );
      })}
    </div>
  );
}

function ConfigPanel({ onSaved }: { onSaved: () => void }) {
  const [model, setModel] = useState("claude-opus-5");
  const [apiKey, setApiKey] = useState("");
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    void ipc.aiGetConfig().then((c) => setModel(c.model));
  }, []);
  return (
    <div style={{ padding: "8px 14px", display: "flex", flexDirection: "column", gap: 12 }}>
      <div style={{ fontSize: 12.5, color: "var(--text-muted)", lineHeight: 1.6 }}>
        配置 Claude API 后,AI 可读取当前会话输出、解释报错并生成命令。Key 经设备密钥加密存储在本地。
      </div>
      <TextInput label="API Key" type="password" placeholder="sk-ant-…" value={apiKey} onChange={(e) => setApiKey(e.target.value)} />
      <TextInput label="模型" value={model} onChange={(e) => setModel(e.target.value)} />
      <Button
        variant="primary"
        disabled={!apiKey.trim()}
        loading={busy}
        onClick={async () => {
          setBusy(true);
          try {
            await ipc.aiSaveConfig(model.trim(), apiKey.trim());
            onSaved();
          } finally {
            setBusy(false);
          }
        }}
      >
        保存并启用
      </Button>
    </div>
  );
}

export function AiDock() {
  const activeId = useSessionStore((s) => s.activeId);
  const [msgs, setMsgs] = useState<Msg[]>([]);
  const [input, setInput] = useState("");
  const [busy, setBusy] = useState(false);
  const [hasKey, setHasKey] = useState<boolean | null>(null);
  const [showConfig, setShowConfig] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    void ipc.aiGetConfig().then((c) => setHasKey(c.hasKey));
  }, []);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [msgs, busy]);

  const send = async () => {
    const q = input.trim();
    if (!q || busy) return;
    if (hasKey === false) {
      setShowConfig(true);
      return;
    }
    const next: Msg[] = [...msgs, { role: "user", text: q }];
    setMsgs(next);
    setInput("");
    setBusy(true);
    try {
      const context = activeId ? getTerminalContext(activeId) : "";
      const reply = await ipc.aiAsk(
        next.map((m) => ({ role: m.role, content: m.text })),
        context,
      );
      setMsgs((m) => [...m, { role: "assistant", text: reply }]);
    } catch (e) {
      const msg = String(e);
      if (msg.includes("AI_KEY_MISSING")) {
        setHasKey(false);
        setShowConfig(true);
        setMsgs(next.slice(0, -1));
        setInput(q);
      } else {
        setMsgs((m) => [...m, { role: "assistant", text: `请求失败:${msg}` }]);
      }
    } finally {
      setBusy(false);
    }
  };

  if (showConfig || hasKey === false) {
    return (
      <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
        <ConfigPanel
          onSaved={() => {
            setHasKey(true);
            setShowConfig(false);
          }}
        />
      </div>
    );
  }

  return (
    <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
      <div ref={scrollRef} className="vscroll" style={{ flex: 1, overflow: "auto", padding: "6px 12px", display: "flex", flexDirection: "column", gap: 10 }}>
        {msgs.length === 0 && (
          <div style={{ fontSize: 12, color: "var(--text-faint)", lineHeight: 1.6, padding: "8px 4px" }}>
            向 AI 提问,或让它解释上一条输出。AI 可读取当前会话最近的终端输出。
          </div>
        )}
        {msgs.map((m, i) =>
          m.role === "user" ? (
            <div key={i} style={{ alignSelf: "flex-end", maxWidth: "88%", background: "var(--accent-subtle)", color: "var(--text-strong)", borderRadius: "12px 12px 4px 12px", padding: "8px 12px", fontSize: 12.5, lineHeight: 1.5, whiteSpace: "pre-wrap" }}>
              {m.text}
            </div>
          ) : (
            <Reply key={i} text={m.text} />
          ),
        )}
        {busy && (
          <div style={{ alignSelf: "flex-start", display: "flex", alignItems: "center", gap: 8, color: "var(--text-faint)", fontSize: 12.5, padding: "4px 2px" }}>
            <Loader2 size={13} style={{ animation: "vspin 0.9s linear infinite" }} />
            思考中…
          </div>
        )}
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "10px 12px", borderTop: "1px solid var(--border-subtle)" }}>
        <div
          onClick={() => setShowConfig(true)}
          className="hover-row"
          title="AI 设置"
          style={{ padding: 6, borderRadius: 8, cursor: "pointer", color: "var(--text-faint)" }}
        >
          <Settings2 size={14} />
        </div>
        <input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && void send()}
          placeholder="询问 AI,或让它解释上一条输出…"
          style={{ flex: 1, background: "var(--surface-sunken)", border: "1px solid var(--border-subtle)", borderRadius: 10, padding: "8px 12px", outline: "none", fontFamily: "var(--font-sans)", fontSize: 12.5, color: "var(--text-strong)" }}
        />
        <div
          onClick={() => void send()}
          style={{ width: 32, height: 32, flex: "none", borderRadius: 10, background: "var(--accent)", color: "var(--text-on-accent)", display: "flex", alignItems: "center", justifyContent: "center", cursor: "pointer", opacity: busy ? 0.5 : 1 }}
        >
          <ArrowUp size={15} />
        </div>
      </div>
      <div style={{ fontSize: 10.5, color: "var(--text-faint)", padding: "0 12px 10px 12px" }}>
        AI 可读取当前会话输出;命令执行前需你确认(点「在终端运行」)。
      </div>
    </div>
  );
}
