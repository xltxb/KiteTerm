// AI 面板:对话与任务两种模式。对话是一问一答,任务是带确认的分步执行。
import { Segmented } from "@/components/ui";
import { useUiStore } from "@/stores/ui";
import { AiChatPane } from "./ai/AiChatPane";
import { AiAgentPane } from "./ai/AiAgentPane";

export function AiDock() {
  const { aiMode, setAiMode } = useUiStore();
  return (
    <div className="col grow" style={{ minHeight: 0 }}>
      <div style={{ padding: "8px 12px 0" }}>
        <Segmented
          options={[
            { value: "chat" as const, label: "对话" },
            { value: "agent" as const, label: "任务" },
          ]}
          value={aiMode}
          onChange={setAiMode}
        />
      </div>
      {aiMode === "chat" ? <AiChatPane /> : <AiAgentPane />}
    </div>
  );
}
