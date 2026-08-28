import { useState } from "react";
import { Button } from "@/components/ui";
import { useSessionStore } from "@/stores/session";

/** AI 回复按 ``` 切段渲染,代码块附带 运行/复制 */
export function Reply({ text }: { text: string }) {
  const runCommand = useSessionStore((s) => s.runCommand);
  const activeId = useSessionStore((s) => s.activeId);
  const [copied, setCopied] = useState<number | null>(null);
  const parts = text.split(/```[a-zA-Z]*\n?/);

  return (
    <div className="col gap-8" style={{ alignSelf: "flex-start", maxWidth: "94%" }}>
      {parts.map((part, i) => {
        const trimmed = part.replace(/\n+$/, "");
        if (!trimmed) return null;
        // 奇数段落是代码块
        if (i % 2 === 1) {
          return (
            <div key={i} className="col gap-6">
              <div className="code-block">{trimmed}</div>
              <div className="row gap-6">
                <Button
                  variant="secondary"
                  size="sm"
                  disabled={!activeId}
                  className="slim"
                  onClick={() => void runCommand(trimmed.trim())}
                >
                  在终端运行
                </Button>
                <Button
                  variant="ghost"
                  size="sm"
                  className="slim"
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
          <div key={i} className="bubble-ai">
            {trimmed}
          </div>
        );
      })}
    </div>
  );
}
