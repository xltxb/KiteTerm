import { useState } from "react";
import { Sparkles } from "lucide-react";
import { readCurrentInputLine, useSessionStore } from "@/stores/session";

/**
 * 终端内的 AI 触发按钮 —— 贴着输入区右下角悬浮。
 * 当前行已经打了字就把那行交给 AI 翻译,空行则打开空的命令栏。
 */
export function AiFab({ sessionId, onOpenEmpty }: { sessionId: string; onOpenEmpty: () => void }) {
  const [hasText, setHasText] = useState(false);

  // 展开(悬停)由 CSS 负责;这里只在悬停那一刻读一次当前行,决定提示文案
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
      className="ai-fab"
      onClick={trigger}
      onMouseEnter={refresh}
      title={hasText ? "把当前这行交给 AI(⌘⇧A)" : "用一句话让 AI 生成命令(⌘⇧A)"}
    >
      <Sparkles size={15} className="fnone" />
      <span className="label">{hasText ? "交给 AI" : "AI 生成命令"}</span>
    </div>
  );
}
