import { useEffect, useRef, useState } from "react";
import {
  AlertTriangle,
  AlignLeft,
  Circle,
  FileUp,
  Lock,
  MessageCircleQuestion,
  PanelRight,
  Plus,
  Radio,
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
import { TransferHint } from "@/screens/terminal/TransferHint";
import { useDropUpload } from "@/screens/terminal/useDropUpload";
import { XtermPane } from "@/screens/terminal/XtermPane";
import { useAiStore } from "@/stores/ai";
import { useAiFeature } from "@/stores/features";
import { getTerminalContext, useSessionStore } from "@/stores/session";
import { useUiStore } from "@/stores/ui";

/** 标签上的连接状态点 */
function stateDot(state: string): string {
  if (state === "connected") return "dot-lg dot-ok";
  if (state === "connecting") return "dot-lg dot-warn";
  return "dot-lg";
}

export function TerminalScreen() {
  const { sessions, activeId, setActive, close, toggleRecording, openLocal, duplicate, mirror, toggleMirror } =
    useSessionStore();
  const { rightPanelOpen, toggleRightPanel, go } = useUiStore();
  const [searchOpen, setSearchOpen] = useState(false);
  const [cmdBarOpen, setCmdBarOpen] = useState(false);
  const [, forceTick] = useState(0);
  const active = sessions.find((s) => s.id === activeId) ?? null;
  const surfaceRef = useRef<HTMLDivElement>(null);
  const dropHover = useDropUpload(surfaceRef, active);
  const setRightTab = useUiStore((s) => s.setRightTab);
  const paletteOpen = useUiStore((s) => s.paletteOpen);
  const explain = useAiStore((s) => s.explain);
  const summarize = useAiStore((s) => s.summarize);
  const pendingAiInput = useSessionStore((s) => s.pendingAiInput);
  const clearPendingAiInput = useSessionStore((s) => s.clearPendingAiInput);
  const setNlEnabled = useSessionStore((s) => s.setNlEnabled);
  const nlSetting = useUiStore((s) => s.settings.naturalLanguageInput);
  const cmdOn = useAiFeature("aiCommand");
  const diagOn = useAiFeature("aiDiagnose");
  const sumOn = useAiFeature("aiSummary");
  const errorHint = useSessionStore((s) => s.errorHint);
  const clearErrorHint = useSessionStore((s) => s.clearErrorHint);
  const setDiagnoseEnabled = useSessionStore((s) => s.setDiagnoseEnabled);
  // 输入镜像的接收方:所有已连接的 SSH 会话(与 lib/broadcast 的口径一致)
  const mirrorIds = mirror ? sessions.filter((s) => s.kind === "ssh" && s.state === "connected").map((s) => s.id) : [];

  // 设置开关同步给终端输入拦截逻辑(白话直连属于「AI 生成命令」子功能)
  useEffect(() => {
    setNlEnabled(nlSetting && cmdOn);
  }, [nlSetting, cmdOn, setNlEnabled]);

  // 「错误诊断提示」子功能门禁同步给会话 store 的检测逻辑
  useEffect(() => {
    setDiagnoseEnabled(diagOn);
  }, [diagOn, setDiagnoseEnabled]);

  /** 把最近输出交给 AI 解释,并把右栏切到 AI 助手 */
  const explainOutput = () => {
    if (!active) return;
    setRightTab("ai");
    if (!useUiStore.getState().rightPanelOpen) toggleRightPanel();
    void explain(getTerminalContext(active.id));
  };

  /** 把最近较长的一段输出交给 AI 摘要,并把右栏切到 AI 助手 */
  const summarizeOutput = () => {
    if (!active) return;
    setRightTab("ai");
    void summarize(getTerminalContext(active.id, 400));
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
      } else if (mod && (e.key === "t" || e.key === "T") && !paletteOpen) {
        e.preventDefault();
        void openLocal();
      } else if (mod && e.shiftKey && (e.key === "b" || e.key === "B")) {
        // ⌘⇧B:开关输入镜像(按键同步到所有 SSH 标签页)
        e.preventDefault();
        toggleMirror();
      } else if (cmdOn && mod && e.shiftKey && (e.key === "a" || e.key === "A")) {
        // ⌘⇧A:自然语言生成命令
        e.preventDefault();
        setCmdBarOpen(true);
      } else if (diagOn && mod && !e.shiftKey && (e.key === "i" || e.key === "I") && active) {
        // ⌘I:让 AI 解释最近的终端输出
        e.preventDefault();
        explainOutput();
      } else if (sumOn && mod && e.shiftKey && (e.key === "s" || e.key === "S") && active) {
        // ⌘⇧S:让 AI 摘要最近较长的一段输出
        e.preventDefault();
        summarizeOutput();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active, close, openLocal, toggleMirror, cmdOn, diagOn, sumOn, paletteOpen]);

  return (
    <>
      <div className="tabbar">
        {sessions.map((s) => (
          <div
            key={s.id}
            className="tab"
            data-active={s.id === activeId ? "true" : "false"}
            data-mirror={mirrorIds.includes(s.id) ? "true" : "false"}
            title="双击复制标签页"
            onClick={() => setActive(s.id)}
            onDoubleClick={() => void duplicate(s.id)}
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
        <div className="tab-action mb-0" title="新建本地终端(⌘T)" onClick={() => void openLocal()}>
          <Plus size={15} />
        </div>
        <div className="spacer" />
        <div
          className="tab-action"
          data-tone={mirror ? "danger" : undefined}
          title={mirror ? "关闭输入镜像(⌘⇧B)" : "输入镜像:按键同步到所有 SSH 标签页(⌘⇧B)"}
          onClick={toggleMirror}
        >
          <Radio size={15} />
        </div>
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
        {cmdOn && (
          <div
            className="tab-action"
            data-tone="accent"
            title="AI 生成命令(⌘⇧A)"
            onClick={() => setCmdBarOpen(true)}
          >
            <Sparkles size={15} />
          </div>
        )}
        {diagOn && active && (
          <div className="tab-action" title="AI 解释最近输出(⌘I)" onClick={explainOutput}>
            <MessageCircleQuestion size={15} />
          </div>
        )}
        {sumOn && active && (
          <div className="tab-action" title="AI 摘要最近输出(⌘⇧S)" onClick={summarizeOutput}>
            <AlignLeft size={15} />
          </div>
        )}
        <div className="tab-action" title="开合右栏" onClick={toggleRightPanel}>
          <PanelRight size={15} />
        </div>
      </div>

      <div className="grow term-body">
        <div className="grow term-surface" ref={surfaceRef}>
          {dropHover && active && (
            <div className="drop-overlay">
              <FileUp size={28} strokeWidth={1.4} />
              <div>松开即通过 ZMODEM 上传到 {active.hostName} 的当前目录</div>
              <div className="fs-xs t-faint">远端需已安装 lrzsz</div>
            </div>
          )}
          {cmdOn && cmdBarOpen && <AiCommandBar onClose={() => setCmdBarOpen(false)} />}
          {cmdOn && pendingAiInput && (
            <AiCommandBar
              initialRequest={pendingAiInput.text}
              replaceLineInSession={pendingAiInput.sessionId}
              onClose={clearPendingAiInput}
            />
          )}
          {active ? (
            <>
              <XtermPane sessionId={active.id} />
              {diagOn && errorHint && errorHint.sessionId === active.id && (
                <div className="error-hint">
                  <AlertTriangle size={13} />
                  <span className="grow ellipsis">检测到命令失败</span>
                  <span className="clickable t-accent" onClick={() => { clearErrorHint(); explainOutput(); }}>AI 诊断</span>
                  <span className="clickable t-faint" onClick={clearErrorHint}>忽略</span>
                </div>
              )}
              <TransferHint sessionId={active.id} />
              {searchOpen && <SearchBar sessionId={active.id} onClose={() => setSearchOpen(false)} />}
              {cmdOn && <AiFab sessionId={active.id} onOpenEmpty={() => setCmdBarOpen(true)} />}
            </>
          ) : (
            <div className="empty-state fill">
              <SquareTerminal size={40} strokeWidth={1.2} />
              <div className="fs-md">没有活跃会话 — 从主机列表连接,或按 ⌘K 快速搜索</div>
              <div className="row gap-8">
                <Button variant="outline" size="sm" onClick={() => void openLocal()}>
                  打开本地终端
                </Button>
                <Button variant="primary" size="sm" onClick={() => go("hosts")}>
                  浏览主机
                </Button>
              </div>
            </div>
          )}
        </div>
        {rightPanelOpen && <RightDock />}
      </div>

      <div className="statusbar">
        {active ? (
          <>
            {active.kind === "local" ? (
              <>
                <span className="grp t-accent">
                  <SquareTerminal size={12} />
                  本地
                </span>
                <span>{active.user}@localhost</span>
              </>
            ) : (
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
              </>
            )}
            <span className="spacer" />
            {mirror && (
              <span className="grp t-danger" title="输入镜像已开启,当前终端的按键会同步到这些标签页">
                <Radio size={12} />
                镜像输入 → {mirrorIds.length} 个标签页
              </span>
            )}
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
