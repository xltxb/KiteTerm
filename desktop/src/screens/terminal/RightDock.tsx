import { useMemo, useState } from "react";
import { Play } from "lucide-react";
import { AiDock } from "@/components/AiDock";
import { CastPlayer } from "@/components/CastPlayer";
import { fmtDuration } from "@/lib/format";
import { useDataStore } from "@/stores/data";
import { useFeaturesStore } from "@/stores/features";
import { useSessionStore } from "@/stores/session";
import { useUiStore } from "@/stores/ui";
import type { Recording } from "@/types";

const TABS = [
  { key: "snip", label: "快捷片段" },
  { key: "rec", label: "录制" },
  { key: "ai", label: "AI 助手" },
] as const;

/** 终端右栏:快捷片段 / 录制 / AI 助手三选一。 */
export function RightDock() {
  const { rightTab, setRightTab } = useUiStore();
  const { snippets, recordings } = useDataStore();
  const runCommand = useSessionStore((s) => s.runCommand);
  const go = useUiStore((s) => s.go);
  const bump = useDataStore((s) => s.bumpSnippet);
  const [playRec, setPlayRec] = useState<Recording | null>(null);
  const aiOn = useFeaturesStore((s) => s.features.ai);
  // AI 关掉时页签消失,若当时正停在 AI 页就退回片段,避免右栏空白
  const tab = rightTab === "ai" && !aiOn ? "snip" : rightTab;

  const quick = useMemo(
    () => [...snippets].sort((a, b) => b.useCount - a.useCount).slice(0, 4),
    [snippets],
  );

  return (
    <div className="dock">
      <div className="dock-seg">
        {TABS.filter((t) => t.key !== "ai" || aiOn).map((t) => (
          <div
            key={t.key}
            className="dock-tab"
            data-active={tab === t.key ? "true" : "false"}
            onClick={() => setRightTab(t.key)}
          >
            {t.label}
          </div>
        ))}
      </div>

      {tab === "snip" && (
        <div className="dock-list vscroll">
          {quick.map((q) => (
            <div
              key={q.id}
              className="snip"
              onClick={() => {
                void runCommand(q.command);
                void bump(q.id);
              }}
            >
              <div className="snip-name">{q.name}</div>
              <div className="snip-cmd">{q.command}</div>
            </div>
          ))}
          <div className="link-more" onClick={() => go("snippets")}>
            管理全部片段 →
          </div>
        </div>
      )}

      {tab === "rec" && (
        <div className="dock-list vscroll">
          {recordings.map((r) => (
            <div key={r.id} className="snip row gap-10" style={{ cursor: "default" }}>
              <div className="round-btn" title="回放" onClick={() => setPlayRec(r)}>
                <Play size={14} />
              </div>
              <div className="grow lh-snug">
                <div className="snip-name ellipsis mb-0">{r.name}</div>
                <div className="fs-xs t-faint">
                  {r.sessionHost} · {r.startedAt}
                </div>
              </div>
              <span className="mono fs-xs t-muted">{fmtDuration(r.durationMs)}</span>
            </div>
          ))}
          <div className="fs-xs t-faint lh-normal" style={{ padding: "4px 2px" }}>
            录制以 asciicast 格式保存,可随会话云同步。
          </div>
        </div>
      )}

      {tab === "ai" && aiOn && <AiDock />}
      {playRec && <CastPlayer recording={playRec} onClose={() => setPlayRec(null)} />}
    </div>
  );
}
