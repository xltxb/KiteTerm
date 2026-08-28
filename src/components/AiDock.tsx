// AI 助手面板 —— 多模型可选;读取当前会话终端输出作为上下文
import { useEffect, useRef, useState } from "react";
import { ArrowUp, Check, Loader2, Pencil, Plus, Settings2, Trash2, X } from "lucide-react";
import { Button, TextInput } from "@/components/ui";
import { getTerminalContext, useSessionStore } from "@/stores/session";
import { useAiStore } from "@/stores/ai";
import type { AiProvider } from "@/types";

/** 常见服务的接入预设 —— 端点与协议是固定的,模型 id 用户可改 */
const PRESETS: { label: string; protocol: AiProvider["protocol"]; baseUrl: string; model: string }[] = [
  { label: "Anthropic Claude", protocol: "anthropic", baseUrl: "https://api.anthropic.com", model: "claude-opus-5" },
  { label: "OpenAI", protocol: "openai", baseUrl: "https://api.openai.com", model: "gpt-4o" },
  { label: "DeepSeek", protocol: "openai", baseUrl: "https://api.deepseek.com", model: "deepseek-chat" },
  { label: "OpenAI 兼容(自填)", protocol: "openai", baseUrl: "", model: "" },
  { label: "Ollama 本地", protocol: "ollama", baseUrl: "http://127.0.0.1:11434", model: "llama3.1" },
];

const PROTOCOL_LABEL: Record<AiProvider["protocol"], string> = {
  anthropic: "Anthropic /v1/messages",
  openai: "OpenAI 兼容 /v1/chat/completions",
  ollama: "Ollama /api/chat",
};

/** 回复按 ``` 切段渲染,代码块附带 运行/复制 */
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
          return (
            <div key={i} style={{ display: "flex", flexDirection: "column", gap: 6 }}>
              <div style={{ background: "#0a0a0b", borderRadius: 10, padding: "9px 12px", fontFamily: "var(--font-mono)", fontSize: 11.5, lineHeight: 1.6, color: "#93edfb", whiteSpace: "pre-wrap", overflowX: "auto" }}>
                {trimmed}
              </div>
              <div style={{ display: "flex", gap: 6 }}>
                <Button variant="secondary" size="sm" disabled={!activeId} style={{ height: 28 }}
                  onClick={() => void runCommand(trimmed.trim())}>
                  在终端运行
                </Button>
                <Button variant="ghost" size="sm" style={{ height: 28 }}
                  onClick={async () => {
                    await navigator.clipboard.writeText(trimmed.trim());
                    setCopied(i);
                    setTimeout(() => setCopied(null), 1500);
                  }}>
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

/** 模型编辑表单 */
function ProviderForm({
  edit,
  onDone,
  onCancel,
}: {
  edit?: AiProvider;
  onDone: () => void;
  onCancel: () => void;
}) {
  const save = useAiStore((s) => s.saveProvider);
  const [form, setForm] = useState<Omit<AiProvider, "hasKey">>(
    edit
      ? { id: edit.id, name: edit.name, protocol: edit.protocol, baseUrl: edit.baseUrl, model: edit.model }
      : { id: "", name: "", protocol: "anthropic", baseUrl: PRESETS[0].baseUrl, model: PRESETS[0].model },
  );
  const [apiKey, setApiKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const set = <K extends keyof typeof form>(k: K, v: (typeof form)[K]) =>
    setForm((f) => ({ ...f, [k]: v }));

  const applyPreset = (label: string) => {
    const p = PRESETS.find((x) => x.label === label);
    if (!p) return;
    setForm((f) => ({
      ...f,
      protocol: p.protocol,
      baseUrl: p.baseUrl,
      model: p.model,
      name: f.name || (p.baseUrl ? p.label : f.name),
    }));
  };

  const needKey = form.protocol !== "ollama";

  return (
    <div style={{ padding: "10px 14px", display: "flex", flexDirection: "column", gap: 10 }}>
      {!edit && (
        <div>
          <label className="input-label">快速选择</label>
          <select className="input" style={{ width: "100%" }} defaultValue={PRESETS[0].label}
            onChange={(e) => applyPreset(e.target.value)}>
            {PRESETS.map((p) => <option key={p.label} value={p.label}>{p.label}</option>)}
          </select>
        </div>
      )}
      <TextInput label="名称" placeholder="给这个模型起个名字" value={form.name}
        onChange={(e) => set("name", e.target.value)} />
      <div>
        <label className="input-label">接口协议</label>
        <select className="input" style={{ width: "100%" }} value={form.protocol}
          onChange={(e) => set("protocol", e.target.value as AiProvider["protocol"])}>
          {(Object.keys(PROTOCOL_LABEL) as AiProvider["protocol"][]).map((k) => (
            <option key={k} value={k}>{PROTOCOL_LABEL[k]}</option>
          ))}
        </select>
      </div>
      <TextInput label="接口地址" placeholder="https://api.example.com" value={form.baseUrl}
        onChange={(e) => set("baseUrl", e.target.value)} />
      <TextInput label="模型 id" placeholder="claude-opus-5 / gpt-4o / llama3.1" value={form.model}
        onChange={(e) => set("model", e.target.value)} />
      {needKey && (
        <TextInput
          label={edit?.hasKey ? "API Key(留空则保持不变)" : "API Key"}
          type="password"
          placeholder={edit?.hasKey ? "••••••••" : "sk-…"}
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
        />
      )}
      {error && <div style={{ fontSize: 12, color: "var(--danger-text)", lineHeight: 1.5 }}>{error}</div>}
      <div style={{ display: "flex", justifyContent: "flex-end", gap: 8 }}>
        <Button variant="ghost" onClick={onCancel}>取消</Button>
        <Button
          variant="primary"
          loading={busy}
          disabled={!form.name.trim() || !form.baseUrl.trim() || !form.model.trim()}
          onClick={async () => {
            setBusy(true);
            setError(null);
            try {
              await save(form, apiKey.trim() === "" ? undefined : apiKey.trim());
              onDone();
            } catch (e) {
              setError(String(e).replace(/^[A-Za-z_]+:\s*/, ""));
            } finally {
              setBusy(false);
            }
          }}
        >
          保存
        </Button>
      </div>
    </div>
  );
}

/** 模型列表 + 增删改切换 */
function ProviderPanel({ onClose }: { onClose: () => void }) {
  const { settings, load, setActive, deleteProvider } = useAiStore();
  const [editing, setEditing] = useState<AiProvider | null>(null);
  const [adding, setAdding] = useState(false);

  useEffect(() => { void load(); }, [load]);

  if (adding || editing) {
    return (
      <div style={{ flex: 1, minHeight: 0, overflowY: "auto" }} className="vscroll">
        <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "10px 14px 0" }}>
          <span style={{ fontSize: 13, fontWeight: 700, color: "var(--text-strong)" }}>
            {editing ? "编辑模型" : "添加模型"}
          </span>
        </div>
        <ProviderForm
          edit={editing ?? undefined}
          onDone={() => { setAdding(false); setEditing(null); }}
          onCancel={() => { setAdding(false); setEditing(null); }}
        />
      </div>
    );
  }

  return (
    <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "10px 14px 6px" }}>
        <span style={{ fontSize: 13, fontWeight: 700, color: "var(--text-strong)", flex: 1 }}>AI 模型</span>
        <div onClick={onClose} className="hover-row" style={{ padding: 5, borderRadius: 7, cursor: "pointer", color: "var(--text-faint)" }}>
          <X size={15} />
        </div>
      </div>
      <div style={{ fontSize: 11.5, color: "var(--text-faint)", padding: "0 14px 8px", lineHeight: 1.6 }}>
        选中的模型用于侧栏对话与终端 AI 协助。密钥经设备密钥加密存本地,不上传。
      </div>
      <div className="vscroll" style={{ flex: 1, overflowY: "auto", padding: "0 14px", display: "flex", flexDirection: "column", gap: 8 }}>
        {settings.providers.length === 0 && (
          <div style={{ fontSize: 12, color: "var(--text-faint)", padding: "10px 2px", lineHeight: 1.6 }}>
            还没有配置模型。点下方「添加模型」,可选 Claude、OpenAI、DeepSeek、任意 OpenAI 兼容服务或本地 Ollama。
          </div>
        )}
        {settings.providers.map((p) => {
          const active = p.id === settings.activeId;
          return (
            <div
              key={p.id}
              onClick={() => void setActive(p.id)}
              style={{
                border: `1px solid ${active ? "var(--accent-subtle-border)" : "var(--border-subtle)"}`,
                background: active ? "var(--accent-subtle)" : "transparent",
                borderRadius: 10,
                padding: "9px 11px",
                cursor: "pointer",
                display: "flex",
                alignItems: "center",
                gap: 8,
              }}
            >
              <div style={{ flex: 1, minWidth: 0, lineHeight: 1.35 }}>
                <div style={{ display: "flex", alignItems: "center", gap: 6 }}>
                  {active && <Check size={12} style={{ color: "var(--accent-text)", flex: "none" }} />}
                  <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--text-strong)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                    {p.name}
                  </span>
                  {!p.hasKey && p.protocol !== "ollama" && (
                    <span style={{ fontSize: 10, color: "var(--warning-text)", flex: "none" }}>缺密钥</span>
                  )}
                </div>
                <div style={{ fontFamily: "var(--font-mono)", fontSize: 10.5, color: "var(--text-faint)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                  {p.model} · {p.protocol}
                </div>
              </div>
              <div className="hover-row" style={{ padding: 4, borderRadius: 6, color: "var(--text-faint)" }}
                onClick={(e) => { e.stopPropagation(); setEditing(p); }}>
                <Pencil size={13} />
              </div>
              <div className="hover-row" style={{ padding: 4, borderRadius: 6, color: "var(--text-faint)" }}
                onClick={(e) => {
                  e.stopPropagation();
                  if (confirm(`删除模型「${p.name}」?`)) void deleteProvider(p.id);
                }}>
                <Trash2 size={13} />
              </div>
            </div>
          );
        })}
      </div>
      <div style={{ padding: "10px 14px" }}>
        <Button variant="secondary" style={{ width: "100%" }} onClick={() => setAdding(true)}>
          <Plus size={14} />添加模型
        </Button>
      </div>
    </div>
  );
}

export function AiDock() {
  const activeId = useSessionStore((s) => s.activeId);
  const { load, activeProvider, messages, busy, send, clear } = useAiStore();
  const [input, setInput] = useState("");
  const [showSettings, setShowSettings] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => { void load(); }, [load]);

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
    <div style={{ flex: 1, minHeight: 0, display: "flex", flexDirection: "column" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 6, padding: "8px 12px 4px" }}>
        <span style={{ fontSize: 11, color: "var(--text-faint)", flex: 1, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
          {provider ? `${provider.name} · ${provider.model}` : "未配置模型"}
        </span>
        {messages.length > 0 && (
          <span onClick={clear} style={{ fontSize: 11, color: "var(--text-faint)", cursor: "pointer" }}>清空</span>
        )}
        <div onClick={() => setShowSettings(true)} className="hover-row" title="AI 模型设置"
          style={{ padding: 4, borderRadius: 6, cursor: "pointer", color: "var(--text-faint)" }}>
          <Settings2 size={14} />
        </div>
      </div>
      <div ref={scrollRef} className="vscroll" style={{ flex: 1, overflow: "auto", padding: "6px 12px", display: "flex", flexDirection: "column", gap: 10 }}>
        {messages.length === 0 && (
          <div style={{ fontSize: 12, color: "var(--text-faint)", lineHeight: 1.7, padding: "8px 4px" }}>
            向 AI 提问,或让它解释上一条输出。
            <br />
            终端内还可以:
            <br />
            <span style={{ fontFamily: "var(--font-mono)", color: "var(--accent-text)" }}>⌘I</span> 解释最近输出 ·{" "}
            <span style={{ fontFamily: "var(--font-mono)", color: "var(--accent-text)" }}>⌘⇧A</span> 用自然语言生成命令
          </div>
        )}
        {messages.map((m, i) =>
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
        <input
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && void doSend()}
          placeholder="询问 AI,或让它解释上一条输出…"
          style={{ flex: 1, background: "var(--surface-sunken)", border: "1px solid var(--border-subtle)", borderRadius: 10, padding: "8px 12px", outline: "none", fontFamily: "var(--font-sans)", fontSize: 12.5, color: "var(--text-strong)" }}
        />
        <div onClick={() => void doSend()}
          style={{ width: 32, height: 32, flex: "none", borderRadius: 10, background: "var(--accent)", color: "var(--text-on-accent)", display: "flex", alignItems: "center", justifyContent: "center", cursor: "pointer", opacity: busy ? 0.5 : 1 }}>
          <ArrowUp size={15} />
        </div>
      </div>
      <div style={{ fontSize: 10.5, color: "var(--text-faint)", padding: "0 12px 10px" }}>
        AI 可读取当前会话输出;命令执行前需你确认(点「在终端运行」)。
      </div>
    </div>
  );
}
