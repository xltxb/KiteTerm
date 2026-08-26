import { ChevronDown, Minus, Plus } from "lucide-react";
import { ScreenHeader, Segmented, Switch } from "@/components/ui";
import { useUiStore } from "@/stores/ui";

const BEHAVIORS = [
  { key: "restoreSessions" as const, label: "启动时恢复会话", desc: "重新打开上次的标签与工作目录" },
  { key: "autoReconnect" as const, label: "断线自动重连", desc: "网络恢复后自动重连 SSH 会话" },
  { key: "confirmOnQuit" as const, label: "退出前确认", desc: "有活跃会话时退出需二次确认" },
];

const SHORTCUTS = [
  { label: "命令面板", keys: "⌘ K" },
  { label: "新建标签", keys: "⌘ T" },
  { label: "关闭标签", keys: "⌘ W" },
  { label: "切换标签", keys: "⌘ 1–9" },
  { label: "拆分终端", keys: "⌘ D" },
  { label: "搜索输出", keys: "⌘ F" },
  { label: "上传文件", keys: "⌘ U" },
];

export function SettingsScreen() {
  const { settings, setSettings } = useUiStore();

  const stepFont = (delta: number) =>
    setSettings({ terminalFontSize: Math.max(11, Math.min(18, settings.terminalFontSize + delta)) });

  return (
    <div className="screen vscroll">
      <div className="screen-inner" style={{ maxWidth: 760 }}>
        <ScreenHeader eyebrow="SETTINGS" title="设置" />

        <div className="card" style={{ padding: "8px 20px", marginBottom: 16 }}>
          <div className="settings-section-title">外观</div>
          <div className="settings-row">
            <span style={{ flex: 1, fontWeight: 600, fontSize: 13.5, color: "var(--text-strong)" }}>主题</span>
            <Segmented
              options={[{ value: "dark", label: "深色" }, { value: "light", label: "浅色" }]}
              value={settings.theme}
              onChange={(theme) => setSettings({ theme })}
            />
          </div>
          <div className="settings-row">
            <span style={{ flex: 1, fontWeight: 600, fontSize: 13.5, color: "var(--text-strong)" }}>终端字体</span>
            <span style={{ fontFamily: "var(--font-mono)", fontSize: 12.5, color: "var(--text-muted)", display: "flex", alignItems: "center", gap: 6, border: "1px solid var(--border-default)", borderRadius: 10, padding: "6px 12px" }}>
              {settings.terminalFont}
              <ChevronDown size={13} />
            </span>
          </div>
          <div className="settings-row">
            <span style={{ flex: 1, fontWeight: 600, fontSize: 13.5, color: "var(--text-strong)" }}>终端字号</span>
            <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
              <div className="hover-row" onClick={() => stepFont(-1)} style={{ width: 26, height: 26, borderRadius: 8, border: "1px solid var(--border-default)", display: "flex", alignItems: "center", justifyContent: "center", cursor: "pointer", color: "var(--text-muted)" }}>
                <Minus size={13} />
              </div>
              <span style={{ fontFamily: "var(--font-mono)", fontSize: 13, color: "var(--text-strong)", width: 38, textAlign: "center" }}>
                {settings.terminalFontSize}px
              </span>
              <div className="hover-row" onClick={() => stepFont(1)} style={{ width: 26, height: 26, borderRadius: 8, border: "1px solid var(--border-default)", display: "flex", alignItems: "center", justifyContent: "center", cursor: "pointer", color: "var(--text-muted)" }}>
                <Plus size={13} />
              </div>
            </div>
          </div>
          <div className="settings-row">
            <span style={{ flex: 1, fontWeight: 600, fontSize: 13.5, color: "var(--text-strong)" }}>光标闪烁</span>
            <Switch on={settings.cursorBlink} onChange={(on) => setSettings({ cursorBlink: on })} />
          </div>
        </div>

        <div className="card" style={{ padding: "8px 20px", marginBottom: 16 }}>
          <div className="settings-section-title">行为</div>
          {BEHAVIORS.map((b) => (
            <div key={b.key} className="settings-row">
              <div style={{ flex: 1, lineHeight: 1.35 }}>
                <div style={{ fontWeight: 600, fontSize: 13.5, color: "var(--text-strong)" }}>{b.label}</div>
                <div style={{ fontSize: 12, color: "var(--text-faint)" }}>{b.desc}</div>
              </div>
              <Switch on={settings[b.key]} onChange={(on) => setSettings({ [b.key]: on })} />
            </div>
          ))}
        </div>

        <div className="card" style={{ padding: "8px 20px" }}>
          <div className="settings-section-title">快捷键</div>
          {SHORTCUTS.map((s) => (
            <div key={s.label} className="settings-row" style={{ padding: "10px 0" }}>
              <span style={{ flex: 1, fontSize: 13, color: "var(--text-strong)" }}>{s.label}</span>
              <span className="kbd">{s.keys}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
