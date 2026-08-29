import { create } from "zustand";
import { Terminal } from "@xterm/xterm";
import { ipc } from "@/ipc";
import { confirmAction } from "@/lib/dialog";
import { looksLikeNaturalLanguage, stripPrompt } from "@/lib/nlDetect";
import { terminalTheme } from "@/theme/terminal";
import type { SessionInfo } from "@/types";

/**
 * xterm 实例不放进 store(非可序列化、更新频繁),模块级注册表按会话 ID 持有;
 * 会话存续期间保留实例,切标签仅重新挂载 DOM,输出缓冲不丢失。
 */
const terminals = new Map<string, Terminal>();

export function getTerminal(id: string): Terminal | undefined {
  return terminals.get(id);
}

/** 主题切换后重新给所有存活的终端上色。
 *
 *  xterm 的 theme 是创建时快照的 CSS 变量取值,不会自己跟着 data-theme 变 ——
 *  后台标签页的实例也在这个表里,所以必须整表刷一遍,不能只刷当前挂载的那个。 */
export function retheme(): void {
  const theme = terminalTheme();
  for (const term of terminals.values()) {
    term.options.theme = theme;
  }
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

/** 读取光标所在行里用户已输入的部分(去掉 shell 提示符) */
export function readCurrentInputLine(id: string): string {
  const term = terminals.get(id);
  if (!term) return "";
  const buf = term.buffer.active;
  const raw = buf.getLine(buf.baseY + buf.cursorY)?.translateToString(true) ?? "";
  return stripPrompt(raw);
}

/** 被拦下的白话输入,等待 AI 处理 */
export interface PendingAiInput {
  sessionId: string;
  text: string;
}

interface SessionState {
  sessions: SessionInfo[];
  activeId: string | null;
  wired: boolean;
  /** 终端里敲了白话并回车 —— 由终端界面接手交给 AI */
  pendingAiInput: PendingAiInput | null;
  /** 运行期开关,由设置同步 */
  nlEnabled: boolean;
  setNlEnabled: (on: boolean) => void;
  clearPendingAiInput: () => void;
  /** 清掉终端当前行(Ctrl+U + Ctrl+K),用于把白话替换成真正的命令 */
  clearInputLine: (id: string) => Promise<void>;
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
  pendingAiInput: null,
  nlEnabled: true,

  setNlEnabled: (on) => set({ nlEnabled: on }),
  clearPendingAiInput: () => set({ pendingAiInput: null }),

  clearInputLine: async (id) => {
    // Ctrl+U 清到行首,Ctrl+K 清到行尾 —— 覆盖光标在中间的情况
    await ipc.writeSession(id, "\x15\x0b");
  },

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
      theme: terminalTheme(),
    });
    term.onData((data) => {
      // 白话直连:光标行是自然语言而非命令时,拦下回车交给 AI
      // (只拦单独的回车键;粘贴进来的整块内容不在此列)
      if (data === "\r" && get().nlEnabled) {
        const line = readCurrentInputLine(info.id);
        if (looksLikeNaturalLanguage(line)) {
          set({ pendingAiInput: { sessionId: info.id, text: line.replace(/^\?\s*/, "") } });
          return; // 回车不下发,原文留在行内 —— 用户取消也不丢输入
        }
      }
      // 粘贴护栏:多行粘贴(含换行的整块输入)需确认,防止误执行。
      // 确认框是异步的(见 lib/dialog.ts),所以要等确认后再下发,不能先写再问。
      const isPaste = data.length > 1 && /[\r\n]/.test(data);
      if (isPaste) {
        const lines = data.split(/\r\n?|\n/).filter((l) => l.length > 0).length;
        const preview = data.slice(0, 160).replace(/[\r\n]+/g, " ⏎ ");
        void confirmAction(
          `将粘贴 ${lines} 行内容到终端(可能立即执行):\n\n${preview}\n\n确认粘贴?`,
          "确认粘贴",
        ).then((ok) => {
          if (ok) void ipc.writeSession(info.id, data);
        });
        return;
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
