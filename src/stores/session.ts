import { create } from "zustand";
import { Terminal } from "@xterm/xterm";
import { ipc } from "@/ipc";
import type { SessionInfo } from "@/types";

/**
 * xterm 实例不放进 store(非可序列化、更新频繁),模块级注册表按会话 ID 持有;
 * 会话存续期间保留实例,切标签仅重新挂载 DOM,输出缓冲不丢失。
 */
const terminals = new Map<string, Terminal>();

export function getTerminal(id: string): Terminal | undefined {
  return terminals.get(id);
}

/** 提取会话终端最近 maxLines 行输出(AI 助手上下文用) */
export function getTerminalContext(id: string, maxLines = 80): string {
  const term = terminals.get(id);
  if (!term) return "";
  const buf = term.buffer.active;
  const end = buf.baseY + buf.cursorY;
  const start = Math.max(0, end - maxLines);
  const lines: string[] = [];
  for (let i = start; i <= end; i++) {
    lines.push(buf.getLine(i)?.translateToString(true) ?? "");
  }
  return lines.join("\n").trim();
}

interface SessionState {
  sessions: SessionInfo[];
  activeId: string | null;
  wired: boolean;
  open: (hostId: string, password?: string) => Promise<string>;
  close: (id: string) => Promise<void>;
  setActive: (id: string) => void;
  runCommand: (command: string) => Promise<void>;
  toggleRecording: (id: string) => Promise<void>;
  wire: () => void;
}

export const useSessionStore = create<SessionState>((set, get) => ({
  sessions: [],
  activeId: null,
  wired: false,

  wire: () => {
    if (get().wired) return;
    set({ wired: true });
    ipc.onSessionData((id, data) => {
      terminals.get(id)?.write(data);
    });
    ipc.onSessionState((info) => {
      set((s) => ({
        sessions: s.sessions.some((x) => x.id === info.id)
          ? s.sessions.map((x) => (x.id === info.id ? info : x))
          : s.sessions,
      }));
    });
  },

  open: async (hostId, password) => {
    get().wire();
    const info = await ipc.openSession(hostId, password);
    const term = new Terminal({
      fontFamily: '"JetBrains Mono", "SF Mono", ui-monospace, Menlo, monospace',
      fontSize: 13,
      lineHeight: 1.25,
      cursorBlink: true,
      scrollback: 10_000,
      theme: {
        background: "#0a0a0b",
        foreground: "#c4ccda",
        cursor: "#2dcde6",
        selectionBackground: "rgba(59,110,246,0.34)",
        black: "#161a27", red: "#ff6259", green: "#2ad07e", yellow: "#ffb648",
        blue: "#5e83fb", magenta: "#9f86fc", cyan: "#5fe1f3", white: "#c4ccda",
        brightBlack: "#5b6478", brightRed: "#ff9d97", brightGreen: "#7be8b3",
        brightYellow: "#ffd391", brightBlue: "#8facff", brightMagenta: "#c4b6ff",
        brightCyan: "#93edfb", brightWhite: "#f4f7fc",
      },
    });
    term.onData((data) => {
      // 粘贴护栏:多行粘贴(含换行的整块输入)需确认,防止误执行
      const isPaste = data.length > 1 && /[\r\n]/.test(data);
      if (isPaste) {
        const lines = data.split(/\r\n?|\n/).filter((l) => l.length > 0).length;
        const preview = data.slice(0, 160).replace(/[\r\n]+/g, " ⏎ ");
        if (!window.confirm(`将粘贴 ${lines} 行内容到终端(可能立即执行):\n\n${preview}\n\n确认粘贴?`)) {
          return;
        }
      }
      void ipc.writeSession(info.id, data);
    });
    term.onResize(({ cols, rows }) => void ipc.resizeSession(info.id, cols, rows));
    terminals.set(info.id, term);
    set((s) => ({ sessions: [...s.sessions, info], activeId: info.id }));
    return info.id;
  },

  close: async (id) => {
    await ipc.closeSession(id);
    terminals.get(id)?.dispose();
    terminals.delete(id);
    set((s) => {
      const sessions = s.sessions.filter((x) => x.id !== id);
      return {
        sessions,
        activeId: s.activeId === id ? (sessions[sessions.length - 1]?.id ?? null) : s.activeId,
      };
    });
  },

  setActive: (id) => set({ activeId: id }),

  runCommand: async (command) => {
    const { activeId } = get();
    if (!activeId) return;
    terminals.get(activeId)?.focus();
    // {{cursor}} 占位符:写入占位符前后的内容,把光标移回占位处停留待编辑,不自动执行
    const marker = "{{cursor}}";
    const at = command.indexOf(marker);
    if (at >= 0) {
      const before = command.slice(0, at);
      const after = command.slice(at + marker.length);
      const left = "\x1b[D".repeat(after.length);
      await ipc.writeSession(activeId, before + after + left);
      return;
    }
    await ipc.writeSession(activeId, command + "\r");
  },

  toggleRecording: async (id) => {
    const s = get().sessions.find((x) => x.id === id);
    if (!s) return;
    if (s.recording) await ipc.stopRecording(id);
    else await ipc.startRecording(id);
  },
}));
