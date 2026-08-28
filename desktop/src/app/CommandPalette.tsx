import { useEffect, useMemo, useRef, useState } from "react";
import {
  Braces,
  KeyRound,
  Moon,
  Plus,
  RefreshCw,
  Search,
  Server,
  Sun,
  Upload,
} from "lucide-react";
import type { ReactNode } from "react";
import { useDataStore } from "@/stores/data";
import { useSessionStore } from "@/stores/session";
import { useSyncStore } from "@/stores/sync";
import { useUiStore } from "@/stores/ui";

interface Cmd {
  id: string;
  icon: ReactNode;
  label: string;
  hint: string;
  keywords: string;
  run: () => void;
}

export function CommandPalette() {
  const { paletteOpen, setPalette, go, settings, setSettings } = useUiStore();
  const { hosts, snippets } = useDataStore();
  const openSession = useSessionStore((s) => s.open);
  const runCommand = useSessionStore((s) => s.runCommand);
  const syncNow = useSyncStore((s) => s.syncNow);
  const [query, setQuery] = useState("");
  const [sel, setSel] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const commands = useMemo<Cmd[]>(() => {
    const dark = settings.theme === "dark";
    const base: Cmd[] = [
      { id: "new-tab", icon: <Plus size={16} />, label: "新建标签页", hint: "⌘T", keywords: "new tab", run: () => go("terminal") },
      { id: "upload", icon: <Upload size={16} />, label: "上传文件…", hint: "SCP", keywords: "upload scp", run: () => go("transfer") },
      { id: "gen-key", icon: <KeyRound size={16} />, label: "生成新密钥", hint: "Ed25519", keywords: "key generate", run: () => go("keys") },
      { id: "sync", icon: <RefreshCw size={16} />, label: "立即同步", hint: "Cloud", keywords: "sync", run: () => { go("sync"); void syncNow(); } },
      {
        id: "theme",
        icon: dark ? <Sun size={16} /> : <Moon size={16} />,
        label: "切换深/浅主题",
        hint: "主题",
        keywords: "theme dark light",
        run: () => setSettings({ theme: dark ? "light" : "dark" }),
      },
    ];
    const hostCmds: Cmd[] = hosts.map((h) => ({
      id: "host-" + h.id,
      icon: <Server size={16} />,
      label: `连接 ${h.name}`,
      hint: "SSH",
      keywords: `${h.name} ${h.addr} ${h.tags.join(" ")} connect ssh`,
      run: () => { go("terminal"); void openSession(h.id); },
    }));
    const snipCmds: Cmd[] = snippets.map((s) => ({
      id: "snip-" + s.id,
      icon: <Braces size={16} />,
      label: `运行片段 ${s.name}`,
      hint: "Snippet",
      keywords: `${s.name} ${s.command} snippet`,
      run: () => { go("terminal"); void runCommand(s.command); },
    }));
    return [...hostCmds, ...base, ...snipCmds];
  }, [hosts, snippets, settings.theme, go, openSession, runCommand, setSettings, syncNow]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    const list = q
      ? commands.filter((c) => (c.label + " " + c.keywords).toLowerCase().includes(q))
      : commands;
    return list.slice(0, 9);
  }, [commands, query]);

  useEffect(() => {
    if (paletteOpen) {
      setQuery("");
      setSel(0);
      setTimeout(() => inputRef.current?.focus(), 30);
    }
  }, [paletteOpen]);

  useEffect(() => setSel(0), [query]);

  if (!paletteOpen) return null;

  return (
    <div
      onClick={() => setPalette(false)}
      className="modal-overlay"
      // 面板从顶部 14vh 落下,层级低于模态,故覆盖 .modal-overlay 的居中与 z-index
      style={{ zIndex: "var(--z-overlay)" as never, alignItems: "flex-start", paddingTop: "14vh" }}
    >
      <div
        onClick={(e) => e.stopPropagation()}
        className="modal lg flush"
      >
        <div className="row gap-10 p-16 bd-bottom">
          <Search size={16} />
          <input
            ref={inputRef}
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") { e.preventDefault(); setSel((s) => Math.min(s + 1, filtered.length - 1)); }
              else if (e.key === "ArrowUp") { e.preventDefault(); setSel((s) => Math.max(s - 1, 0)); }
              else if (e.key === "Enter" && filtered[sel]) { filtered[sel].run(); setPalette(false); }
            }}
            placeholder="输入命令、主机或片段…"
            className="input-bare fs-md"
          />
          <span className="kbd fs-2xs t-faint">ESC</span>
        </div>
        <div className="col p-8">
          {filtered.map((c, i) => (
            <div
              key={c.id}
              onClick={() => { c.run(); setPalette(false); }}
              onMouseEnter={() => setSel(i)}
              className="row gap-12 clickable r-md p-10"
              // 选中项高亮由键盘/悬停状态决定,保留内联
              style={{ background: i === sel ? "var(--accent-subtle)" : "transparent" }}
            >
              {c.icon}
              <span className="grow fs-md t-strong">{c.label}</span>
              <span className="mono fs-xs t-faint">{c.hint}</span>
            </div>
          ))}
          {filtered.length === 0 && (
            <div className="empty-state fs-md p-16">
              没有匹配的命令
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
