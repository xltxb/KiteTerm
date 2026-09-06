// AI 助手面板 —— 多模型可选;读取当前会话终端输出作为上下文
import { useEffect, useRef, useState } from "react";
import { ArrowUp, Loader2, Settings2 } from "lucide-react";
import { ProviderPanel } from "@/components/ai/ProviderPanel";
import { Reply } from "@/components/ai/Reply";
import { useAiFeature } from "@/stores/features";
import { getTerminalContext, useSessionStore } from "@/stores/session";
import { useAiStore } from "@/stores/ai";

export function AiDock() {
  const activeId = useSessionStore((s) => s.activeId);
  const { load, activeProvider, messages, busy, send, clear } = useAiStore();
  const diagOn = useAiFeature("aiDiagnose");
  const sumOn = useAiFeature("aiSummary");
  const cmdOn = useAiFeature("aiCommand");
  const [input, setInput] = useState("");
  const [showSettings, setShowSettings] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    void load();
  }, [load]);

  useEffect(() => {
    scrollRef.current?.scrollTo({ top: scrollRef.current.scrollHeight });
  }, [messages, busy]);

  const provider = activeProvider();

  const doSend = async () => {
    const q = input.trim();
    if (!q || busy) return;
    if (!provider) {
      setShowSettings(true);
      return;
    }
    setInput("");
    await send(q, activeId ? getTerminalContext(activeId) : "");
  };

  if (showSettings || (!provider && messages.length === 0)) {
    return <ProviderPanel onClose={() => setShowSettings(false)} />;
  }

  return (
    <div className="panel">
      <div className="row gap-6 chat-head">
        <span className="fs-xs t-faint grow ellipsis">
          {provider ? `${provider.name} · ${provider.model}` : "未配置模型"}
        </span>
        {messages.length > 0 && (
          <span className="fs-xs t-faint clickable" onClick={clear}>清空</span>
        )}
        <div className="icon-action sm" title="AI 模型设置" onClick={() => setShowSettings(true)}>
          <Settings2 size={14} />
        </div>
      </div>

      <div ref={scrollRef} className="chat-list vscroll">
        {messages.length === 0 && (
          <div className="fs-sm t-faint lh-relaxed" style={{ padding: "8px 4px" }}>
            向 AI 提问,或让它解释上一条输出。
            {(diagOn || sumOn || cmdOn) && (
              <>
                <br />
                终端内还可以:
                <br />
                <span className="mono t-accent">
                  {diagOn && "⌘I 解释最近输出"}
                  {sumOn && " · ⌘⇧S 摘要长输出"}
                  {cmdOn && " · ⌘⇧A 生成命令"}
                </span>
              </>
            )}
          </div>
        )}
        {messages.map((m, i) =>
          m.role === "user" ? (
            <div key={i} className="bubble-user">{m.text}</div>
          ) : (
            <Reply key={i} text={m.text} />
          ),
        )}
        {busy && (
          <div
            className="row gap-8 fs-sm t-faint"
            style={{ alignSelf: "flex-start", padding: "4px 2px" }}
          >
            <Loader2 size={13} className="spin" />
            思考中…
          </div>
        )}
      </div>

      <div className="chat-input-row">
        <input
          className="chat-input"
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && void doSend()}
          placeholder="询问 AI,或让它解释上一条输出…"
        />
        <div className="send-btn" data-busy={busy ? "true" : "false"} onClick={() => void doSend()}>
          <ArrowUp size={15} />
        </div>
      </div>
      <div className="fs-2xs t-faint chat-foot">
        AI 可读取当前会话输出;命令执行前需你确认(点「在终端运行」)。
      </div>
    </div>
  );
}
