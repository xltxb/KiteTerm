import { useEffect, useMemo, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";
import { WebglAddon } from "@xterm/addon-webgl";
import {
  Circle,
  Lock,
  MessageCircleQuestion,
  PanelRight,
  Play,
  Plus,
  Search,
  Sparkles,
  SquareTerminal,
  X,
} from "lucide-react";
import { Button } from "@/components/ui";
import { AiCommandBar } from "@/components/AiCommandBar";
import { AiDock } from "@/components/AiDock";
import { CastPlayer } from "@/components/CastPlayer";
import { useAiStore } from "@/stores/ai";
import { useDataStore } from "@/stores/data";
import {
  getTerminal,
  getTerminalContext,
  readCurrentInputLine,
  useSessionStore,
} from "@/stores/session";
import { useUiStore } from "@/stores/ui";
import type { Recording } from "@/types";

interface Addons {
  fit: FitAddon;
  search: SearchAddon;
  opened: boolean;
}
const addonMap = new WeakMap<Terminal, Addons>();

function ensureAddons(term: Terminal): Addons {
  let a = addonMap.get(term);
  if (!a) {
    a = { fit: new FitAddon(), search: new SearchAddon(), opened: false };
    term.loadAddon(a.fit);
    term.loadAddon(a.search);
    addonMap.set(term, a);
  }
  return a;
}

function fmtDuration(ms: number): string {
  const s = Math.floor(ms / 1000);
  const m = Math.floor(s / 60);
  return `${String(m).padStart(2, "0")}:${String(s % 60).padStart(2, "0")}`;
}

function XtermPane({ sessionId }: { sessionId: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const { settings } = useUiStore();

  useEffect(() => {
    const el = ref.current;
    const term = getTerminal(sessionId);
    if (!el || !term) return;
    const addons = ensureAddons(term);
    if (!addons.opened) {
      term.open(el);
      addons.opened = true;
      try {
        term.loadAddon(new WebglAddon());
      } catch {
        // WebGL 不可用时回退 canvas 渲染
      }
    } else if (term.element && term.element.parentElement !== el) {
      el.appendChild(term.element);
    }
    addons.fit.fit();
    term.focus();
    const ro = new ResizeObserver(() => addons.fit.fit());
    ro.observe(el);
    return () => ro.disconnect();
  }, [sessionId]);

  useEffect(() => {
    const term = getTerminal(sessionId);
    if (!term) return;
    term.options.fontSize = settings.terminalFontSize;
    term.options.cursorBlink = settings.cursorBlink;
    addonMap.get(term)?.fit.fit();
  }, [sessionId, settings.terminalFontSize, settings.cursorBlink]);

  return <div ref={ref} className="vterm" style={{ position: "absolute", inset: 0, padding: "10px 4px 4px 14px", background: "#0a0a0b" }} />;
}

/**
 * 终端内的 AI 触发按钮 —— 贴着输入区右下角悬浮。
 * 当前行已经打了字就把那行交给 AI 翻译,空行则打开空的命令栏。
 */
function AiFab({ sessionId, onOpenEmpty }: { sessionId: string; onOpenEmpty: () => void }) {
  const [hover, setHover] = useState(false);
  const [hasText, setHasText] = useState(false);

  // 悬停时才读当前行,决定提示文案(读 buffer 很轻量,但没必要每帧做)
  const refresh = () => setHasText(readCurrentInputLine(sessionId).trim().length > 0);

  const trigger = () => {
    const line = readCurrentInputLine(sessionId).trim();
    if (line) {
      // 复用白话直连的流程:自动翻译并在采纳后替换该行
      useSessionStore.setState({
        pendingAiInput: { sessionId, text: line.replace(/^\?\s*/, "") },
      });
    } else {
      onOpenEmpty();
    }
  };

  return (
    <div
      onClick={trigger}
      onMouseEnter={() => { setHover(true); refresh(); }}
      onMouseLeave={() => setHover(false)}
      title={hasText ? "把当前这行交给 AI(⌘⇧A)" : "用一句话让 AI 生成命令(⌘⇧A)"}
      style={{
        position: "absolute",
        right: 16,
        bottom: 14,
        zIndex: 12,
        height: 32,
        display: "flex",
        alignItems: "center",
        gap: 6,
        padding: hover ? "0 12px" : 0,
        width: hover ? "auto" : 32,
        justifyContent: "center",
        borderRadius: 99,
        cursor: "pointer",
        color: hover ? "var(--text-on-accent)" : "var(--accent-text)",
        background: hover ? "var(--accent)" : "var(--accent-subtle)",
        border: `1px solid ${hover ? "var(--accent)" : "var(--accent-subtle-border)"}`,
        boxShadow: hover ? "var(--glow-sm)" : "none",
        transition: "var(--transition-colors), padding 140ms var(--ease-out)",
        userSelect: "none",
        whiteSpace: "nowrap",
      }}
    >
      <Sparkles size={15} style={{ flex: "none" }} />
      {hover && (
        <span style={{ fontSize: 12, fontWeight: 600 }}>
          {hasText ? "交给 AI" : "AI 生成命令"}
        </span>
      )}
    </div>
  );
}

function SearchBar({ sessionId, onClose }: { sessionId: string; onClose: () => void }) {
  const [q, setQ] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => inputRef.current?.focus(), []);
  const term = getTerminal(sessionId);
  const search = term ? ensureAddons(term).search : null;
  return (
    <div style={{ position: "absolute", top: 10, right: 14, zIndex: 10, display: "flex", alignItems: "center", gap: 6, background: "var(--surface-raised)", border: "1px solid var(--border-default)", borderRadius: 10, padding: "6px 10px", boxShadow: "var(--shadow-lg)" }}>
      <Search size={14} />
      <input
        ref={inputRef}
        value={q}
        onChange={(e) => { setQ(e.target.value); search?.findNext(e.target.value); }}
        onKeyDown={(e) => {
          if (e.key === "Enter") search?.findNext(q);
          if (e.key === "Escape") onClose();
        }}
        placeholder="搜索输出…"
        style={{ background: "transparent", border: "none", outline: "none", fontSize: 12.5, color: "var(--text-strong)", width: 160 }}
      />
      <X size={14} style={{ cursor: "pointer" }} onClick={onClose} />
    </div>
  );
}

function RightDock() {
  const { rightTab, setRightTab } = useUiStore();
  const { snippets, recordings } = useDataStore();
  const runCommand = useSessionStore((s) => s.runCommand);
  const go = useUiStore((s) => s.go);
  const bump = useDataStore((s) => s.bumpSnippet);
  const [playRec, setPlayRec] = useState<Recording | null>(null);

  const quick = useMemo(
    () => [...snippets].sort((a, b) => b.useCount - a.useCount).slice(0, 4),
    [snippets],
  );

  const tabBtn = (key: "snip" | "rec" | "ai", label: string) => (
    <div
      onClick={() => setRightTab(key)}
      style={{
        flex: 1,
        textAlign: "center",
        padding: "6px 0",
        borderRadius: 99,
        fontSize: 12.5,
        fontWeight: 600,
        cursor: "pointer",
        color: rightTab === key ? "var(--accent-text)" : "var(--text-muted)",
        background: rightTab === key ? "var(--accent-subtle)" : "transparent",
      }}
    >
      {label}
    </div>
  );

  return (
    <div style={{ width: 284, flex: "none", borderLeft: "1px solid var(--border-subtle)", background: "var(--surface-card)", display: "flex", flexDirection: "column" }}>
      <div style={{ display: "flex", gap: 4, padding: "12px 12px 8px 12px" }}>
        {tabBtn("snip", "快捷片段")}
        {tabBtn("rec", "录制")}
        {tabBtn("ai", "AI 助手")}
      </div>

      {rightTab === "snip" && (
        <div className="vscroll" style={{ flex: 1, overflow: "auto", padding: "4px 12px 12px 12px", display: "flex", flexDirection: "column", gap: 8 }}>
          {quick.map((q) => (
            <div
              key={q.id}
              className="hover-card"
              onClick={() => { void runCommand(q.command); void bump(q.id); }}
              style={{ border: "1px solid var(--border-subtle)", borderRadius: 10, padding: "10px 12px", cursor: "pointer" }}
            >
              <div style={{ fontSize: 12.5, fontWeight: 600, color: "var(--text-strong)", marginBottom: 4 }}>{q.name}</div>
              <div style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--text-muted)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                {q.command}
              </div>
            </div>
          ))}
          <div onClick={() => go("snippets")} style={{ textAlign: "center", fontSize: 12, color: "var(--accent-text)", cursor: "pointer", padding: 6 }}>
            管理全部片段 →
          </div>
        </div>
      )}

      {rightTab === "rec" && (
        <div className="vscroll" style={{ flex: 1, overflow: "auto", padding: "4px 12px 12px 12px", display: "flex", flexDirection: "column", gap: 8 }}>
          {recordings.map((r) => (
            <div key={r.id} className="hover-card" style={{ border: "1px solid var(--border-subtle)", borderRadius: 10, padding: "10px 12px", display: "flex", alignItems: "center", gap: 10 }}>
              <div
                onClick={() => setPlayRec(r)}
                title="回放"
                style={{ width: 30, height: 30, flex: "none", borderRadius: 99, background: "var(--accent-subtle)", color: "var(--accent-text)", display: "flex", alignItems: "center", justifyContent: "center", cursor: "pointer" }}
              >
                <Play size={14} />
              </div>
              <div style={{ flex: 1, minWidth: 0, lineHeight: 1.3 }}>
                <div style={{ fontSize: 12.5, fontWeight: 600, color: "var(--text-strong)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{r.name}</div>
                <div style={{ fontSize: 11, color: "var(--text-faint)" }}>{r.sessionHost} · {r.startedAt}</div>
              </div>
              <span style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--text-muted)" }}>{fmtDuration(r.durationMs)}</span>
            </div>
          ))}
          <div style={{ fontSize: 11.5, color: "var(--text-faint)", padding: "4px 2px", lineHeight: 1.5 }}>
            录制以 asciicast 格式保存,可随会话云同步。
          </div>
        </div>
      )}

      {rightTab === "ai" && <AiDock />}
      {playRec && <CastPlayer recording={playRec} onClose={() => setPlayRec(null)} />}
    </div>
  );
}

export function TerminalScreen() {
  const { sessions, activeId, setActive, close, toggleRecording } = useSessionStore();
  const { rightPanelOpen, toggleRightPanel, setPalette, go } = useUiStore();
  const [searchOpen, setSearchOpen] = useState(false);
  const [cmdBarOpen, setCmdBarOpen] = useState(false);
  const [, forceTick] = useState(0);
  const active = sessions.find((s) => s.id === activeId) ?? null;
  const setRightTab = useUiStore((s) => s.setRightTab);
  const explain = useAiStore((s) => s.explain);
  const pendingAiInput = useSessionStore((s) => s.pendingAiInput);
  const clearPendingAiInput = useSessionStore((s) => s.clearPendingAiInput);
  const setNlEnabled = useSessionStore((s) => s.setNlEnabled);
  const nlSetting = useUiStore((s) => s.settings.naturalLanguageInput);

  // 设置开关同步给终端输入拦截逻辑
  useEffect(() => { setNlEnabled(nlSetting); }, [nlSetting, setNlEnabled]);

  /** 把最近输出交给 AI 解释,并把右栏切到 AI 助手 */
  const explainOutput = () => {
    if (!active) return;
    setRightTab("ai");
    if (!useUiStore.getState().rightPanelOpen) toggleRightPanel();
    void explain(getTerminalContext(active.id));
  };

  // REC 计时与延迟展示每秒刷新
  useEffect(() => {
    const t = setInterval(() => forceTick((x) => x + 1), 1000);
    return () => clearInterval(t);
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && (e.key === "f" || e.key === "F") && active) {
        e.preventDefault();
        setSearchOpen((s) => !s);
      } else if (mod && (e.key === "w" || e.key === "W") && active) {
        e.preventDefault();
        void close(active.id);
      } else if (mod && (e.key === "t" || e.key === "T")) {
        e.preventDefault();
        setPalette(true);
      } else if (mod && e.shiftKey && (e.key === "a" || e.key === "A")) {
        // ⌘⇧A:自然语言生成命令
        e.preventDefault();
        setCmdBarOpen(true);
      } else if (mod && !e.shiftKey && (e.key === "i" || e.key === "I") && active) {
        // ⌘I:让 AI 解释最近的终端输出
        e.preventDefault();
        explainOutput();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, close, setPalette]);

  const dotColor = (state: string, isActive: boolean) =>
    state === "connected" ? (isActive ? "var(--success)" : "var(--success)") :
    state === "connecting" ? "var(--warning)" : "var(--text-faint)";

  return (
    <>
      <div style={{ display: "flex", alignItems: "center", gap: 6, padding: "8px 12px 0 12px", background: "var(--surface-card)", borderBottom: "1px solid var(--border-subtle)" }}>
        {sessions.map((s) => (
          <div
            key={s.id}
            onClick={() => setActive(s.id)}
            style={{
              display: "flex",
              alignItems: "center",
              gap: 8,
              padding: "8px 14px",
              borderRadius: "10px 10px 0 0",
              cursor: "pointer",
              fontSize: 13,
              fontWeight: 600,
              color: s.id === activeId ? "var(--text-strong)" : "var(--text-muted)",
              background: s.id === activeId ? "var(--surface-sunken)" : "transparent",
              boxShadow: s.id === activeId ? "inset 0 2px 0 var(--accent)" : "none",
            }}
          >
            <span style={{ width: 7, height: 7, borderRadius: 99, background: dotColor(s.state, s.id === activeId) }} />
            {s.hostName}
            <X
              size={12}
              style={{ opacity: 0.6 }}
              onClick={(e) => { e.stopPropagation(); void close(s.id); }}
            />
          </div>
        ))}
        <div onClick={() => setPalette(true)} className="hover-row" style={{ padding: "6px 8px", borderRadius: 8, cursor: "pointer", color: "var(--text-muted)" }}>
          <Plus size={15} />
        </div>
        <div style={{ flex: 1 }} />
        {active && (
          <div
            onClick={() => void toggleRecording(active.id)}
            className="hover-row"
            title={active.recording ? "停止录制" : "开始录制"}
            style={{ padding: "6px 8px", marginBottom: 4, borderRadius: 8, cursor: "pointer", color: active.recording ? "var(--danger)" : "var(--text-muted)" }}
          >
            <Circle size={15} fill={active.recording ? "currentColor" : "none"} />
          </div>
        )}
        <div
          onClick={() => setCmdBarOpen(true)}
          className="hover-row"
          title="AI 生成命令(⌘⇧A)"
          style={{ padding: "6px 8px", marginBottom: 4, borderRadius: 8, cursor: "pointer", color: "var(--accent-text)" }}
        >
          <Sparkles size={15} />
        </div>
        {active && (
          <div
            onClick={explainOutput}
            className="hover-row"
            title="AI 解释最近输出(⌘I)"
            style={{ padding: "6px 8px", marginBottom: 4, borderRadius: 8, cursor: "pointer", color: "var(--text-muted)" }}
          >
            <MessageCircleQuestion size={15} />
          </div>
        )}
        <div onClick={toggleRightPanel} className="hover-row" style={{ padding: "6px 8px", marginBottom: 4, borderRadius: 8, cursor: "pointer", color: "var(--text-muted)" }}>
          <PanelRight size={15} />
        </div>
      </div>

      <div style={{ flex: 1, minHeight: 0, display: "flex" }}>
        <div style={{ flex: 1, minWidth: 0, position: "relative", background: "#0a0a0b" }}>
          {cmdBarOpen && <AiCommandBar onClose={() => setCmdBarOpen(false)} />}
          {pendingAiInput && (
            <AiCommandBar
              initialRequest={pendingAiInput.text}
              replaceLineInSession={pendingAiInput.sessionId}
              onClose={clearPendingAiInput}
            />
          )}
          {active ? (
            <>
              <XtermPane sessionId={active.id} />
              {searchOpen && <SearchBar sessionId={active.id} onClose={() => setSearchOpen(false)} />}
              <AiFab sessionId={active.id} onOpenEmpty={() => setCmdBarOpen(true)} />
            </>
          ) : (
            <div style={{ position: "absolute", inset: 0, display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", gap: 14, color: "var(--text-faint)" }}>
              <SquareTerminal size={40} strokeWidth={1.2} />
              <div style={{ fontSize: 13.5 }}>没有活跃会话 — 从主机列表连接,或按 ⌘K 快速搜索</div>
              <Button variant="primary" size="sm" onClick={() => go("hosts")}>
                浏览主机
              </Button>
            </div>
          )}
        </div>
        {rightPanelOpen && <RightDock />}
      </div>

      <div style={{ display: "flex", alignItems: "center", gap: 16, padding: "7px 16px", background: "var(--surface-card)", borderTop: "1px solid var(--border-subtle)", fontFamily: "var(--font-mono)", fontSize: 11.5, color: "var(--text-muted)" }}>
        {active ? (
          <>
            <span style={{ display: "flex", alignItems: "center", gap: 6, color: "var(--accent-text)" }}>
              <Lock size={12} />SSH
            </span>
            <span>{active.user}@{active.addr}:{active.port}</span>
            <span style={{ display: "flex", alignItems: "center", gap: 6 }}>
              <span style={{ width: 6, height: 6, borderRadius: 99, background: active.state === "connected" ? "var(--success)" : "var(--warning)" }} />
              {active.latencyMs != null ? `${active.latencyMs}ms` : "—"}
            </span>
            <span>{active.cipher}</span>
            <span style={{ flex: 1 }} />
            {active.recording && active.recordingSince && (
              <span style={{ display: "flex", alignItems: "center", gap: 6, color: "var(--danger)" }}>
                <span style={{ width: 7, height: 7, borderRadius: 99, background: "var(--danger)", animation: "vblink 1.4s steps(1) infinite" }} />
                REC {fmtDuration(Date.now() - active.recordingSince)}
              </span>
            )}
            <span>UTF-8</span>
            <span>{active.cols}×{active.rows}</span>
          </>
        ) : (
          <span style={{ color: "var(--text-faint)" }}>未连接</span>
        )}
      </div>
    </>
  );
}
