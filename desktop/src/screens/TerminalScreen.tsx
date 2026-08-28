import { useEffect, useState } from "react";
import {
  Circle,
  Lock,
  MessageCircleQuestion,
  PanelRight,
  Plus,
  Sparkles,
  SquareTerminal,
  X,
} from "lucide-react";
import { Button } from "@/components/ui";
import { AiCommandBar } from "@/components/AiCommandBar";
import { fmtDuration } from "@/lib/format";
import { AiFab } from "@/screens/terminal/AiFab";
import { RightDock } from "@/screens/terminal/RightDock";
import { SearchBar } from "@/screens/terminal/SearchBar";
import { XtermPane } from "@/screens/terminal/XtermPane";
import { useAiStore } from "@/stores/ai";
import { useFeaturesStore } from "@/stores/features";
import { getTerminalContext, useSessionStore } from "@/stores/session";
import { useUiStore } from "@/stores/ui";

/** 标签上的连接状态点 */
function stateDot(state: string): string {
  if (state === "connected") return "dot-lg dot-ok";
  if (state === "connecting") return "dot-lg dot-warn";
  return "dot-lg";
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
  const aiOn = useFeaturesStore((s) => s.features.ai);

  // 设置开关同步给终端输入拦截逻辑
  useEffect(() => {
    setNlEnabled(nlSetting && aiOn);
  }, [nlSetting, aiOn, setNlEnabled]);

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
      } else if (aiOn && mod && e.shiftKey && (e.key === "a" || e.key === "A")) {
        // ⌘⇧A:自然语言生成命令
        e.preventDefault();
        setCmdBarOpen(true);
      } else if (aiOn && mod && !e.shiftKey && (e.key === "i" || e.key === "I") && active) {
        // ⌘I:让 AI 解释最近的终端输出
        e.preventDefault();
        explainOutput();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, close, setPalette, aiOn]);

  return (
    <>
      <div className="tabbar">
        {sessions.map((s) => (
          <div
            key={s.id}
            className="tab"
            data-active={s.id === activeId ? "true" : "false"}
            onClick={() => setActive(s.id)}
          >
            <span className={stateDot(s.state)} />
            {s.hostName}
            <X
              size={12}
              style={{ opacity: 0.6 }}
              onClick={(e) => {
                e.stopPropagation();
                void close(s.id);
              }}
            />
          </div>
        ))}
        <div className="tab-action mb-0" onClick={() => setPalette(true)}>
          <Plus size={15} />
        </div>
        <div className="spacer" />
        {active && (
          <div
            className="tab-action"
            data-tone={active.recording ? "danger" : undefined}
            title={active.recording ? "停止录制" : "开始录制"}
            onClick={() => void toggleRecording(active.id)}
          >
            <Circle size={15} fill={active.recording ? "currentColor" : "none"} />
          </div>
        )}
        {aiOn && (
          <div
            className="tab-action"
            data-tone="accent"
            title="AI 生成命令(⌘⇧A)"
            onClick={() => setCmdBarOpen(true)}
          >
            <Sparkles size={15} />
          </div>
        )}
        {aiOn && active && (
          <div className="tab-action" title="AI 解释最近输出(⌘I)" onClick={explainOutput}>
            <MessageCircleQuestion size={15} />
          </div>
        )}
        <div className="tab-action" title="开合右栏" onClick={toggleRightPanel}>
          <PanelRight size={15} />
        </div>
      </div>

      <div className="grow term-body">
        <div className="grow term-surface">
          {aiOn && cmdBarOpen && <AiCommandBar onClose={() => setCmdBarOpen(false)} />}
          {aiOn && pendingAiInput && (
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
              {aiOn && <AiFab sessionId={active.id} onOpenEmpty={() => setCmdBarOpen(true)} />}
            </>
          ) : (
            <div className="empty-state fill">
              <SquareTerminal size={40} strokeWidth={1.2} />
              <div className="fs-md">没有活跃会话 — 从主机列表连接,或按 ⌘K 快速搜索</div>
              <Button variant="primary" size="sm" onClick={() => go("hosts")}>
                浏览主机
              </Button>
            </div>
          )}
        </div>
        {rightPanelOpen && <RightDock />}
      </div>

      <div className="statusbar">
        {active ? (
          <>
            <span className="grp t-accent">
              <Lock size={12} />
              SSH
            </span>
            <span>
              {active.user}@{active.addr}:{active.port}
            </span>
            <span className="grp">
              <span className={active.state === "connected" ? "dot-sm dot-ok" : "dot-sm dot-warn"} />
              {active.latencyMs != null ? `${active.latencyMs}ms` : "—"}
            </span>
            <span>{active.cipher}</span>
            <span className="spacer" />
            {active.recording && active.recordingSince && (
              <span className="grp t-danger">
                <span className="dot-lg dot-rec" />
                REC {fmtDuration(Date.now() - active.recordingSince)}
              </span>
            )}
            <span>UTF-8</span>
            <span>
              {active.cols}×{active.rows}
            </span>
          </>
        ) : (
          <span className="t-faint">未连接</span>
        )}
      </div>
    </>
  );
}
