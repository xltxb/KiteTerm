import { ChevronDown, Minus, Plus } from "lucide-react";
import { ScreenHeader, Segmented, Switch } from "@/components/ui";
import { useUiStore } from "@/stores/ui";

const BEHAVIORS = [
  { key: "restoreSessions" as const, label: "启动时恢复会话", desc: "重新打开上次的标签与工作目录" },
  { key: "autoReconnect" as const, label: "断线自动重连", desc: "网络恢复后自动重连 SSH 会话" },
  { key: "confirmOnQuit" as const, label: "退出前确认", desc: "有活跃会话时退出需二次确认" },
  {
    key: "naturalLanguageInput" as const,
    label: "终端白话直连 AI",
    desc: "在终端直接敲自然语言回车,自动交给 AI 翻译成命令(采纳前需确认)",
  },
];

const SHORTCUTS = [
  { label: "命令面板", keys: "⌘ K" },
  { label: "新建标签", keys: "⌘ T" },
  { label: "关闭标签", keys: "⌘ W" },
  { label: "切换标签", keys: "⌘ 1–9" },
  { label: "拆分终端", keys: "⌘ D" },
  { label: "搜索输出", keys: "⌘ F" },
  { label: "上传文件", keys: "⌘ U" },
  { label: "AI 解释最近输出", keys: "⌘ I" },
  { label: "AI 生成命令", keys: "⌘ ⇧ A" },
];

export function SettingsScreen() {
  const { settings, setSettings } = useUiStore();

  const stepFont = (delta: number) =>
    setSettings({ terminalFontSize: Math.max(11, Math.min(18, settings.terminalFontSize + delta)) });

  return (
    <div className="screen vscroll">
      <div className="screen-inner narrow">
        <ScreenHeader eyebrow="SETTINGS" title="设置" />

        <div className="card pad-band mb-16">
          <div className="settings-section-title">外观</div>
          <div className="settings-row">
            <span className="grow w-semi fs-md t-strong">主题</span>
            <Segmented
              options={[{ value: "dark", label: "深色" }, { value: "light", label: "浅色" }]}
              value={settings.theme}
              onChange={(theme) => setSettings({ theme })}
            />
          </div>
          <div className="settings-row">
            <span className="grow w-semi fs-md t-strong">终端字体</span>
            <span className="row gap-6 mono fs-sm t-muted r-md bd py-6 px-12">
              {settings.terminalFont}
              <ChevronDown size={13} />
            </span>
          </div>
          <div className="settings-row">
            <span className="grow w-semi fs-md t-strong">终端字号</span>
            <div className="row gap-10">
              <div className="icon-btn sm" onClick={() => stepFont(-1)}>
                <Minus size={13} />
              </div>
              <span className="mono fs-md t-strong text-center num-readout">
                {settings.terminalFontSize}px
              </span>
              <div className="icon-btn sm" onClick={() => stepFont(1)}>
                <Plus size={13} />
              </div>
            </div>
          </div>
          <div className="settings-row">
            <span className="grow w-semi fs-md t-strong">光标闪烁</span>
            <Switch on={settings.cursorBlink} onChange={(on) => setSettings({ cursorBlink: on })} />
          </div>
        </div>

        <div className="card pad-band mb-16">
          <div className="settings-section-title">行为</div>
          {BEHAVIORS.map((b) => (
            <div key={b.key} className="settings-row">
              <div className="grow lh-snug">
                <div className="w-semi fs-md t-strong">{b.label}</div>
                <div className="fs-sm t-faint">{b.desc}</div>
              </div>
              <Switch on={settings[b.key]} onChange={(on) => setSettings({ [b.key]: on })} />
            </div>
          ))}
        </div>

        <div className="card pad-band">
          <div className="settings-section-title">快捷键</div>
          {SHORTCUTS.map((s) => (
            <div key={s.label} className="settings-row">
              <span className="grow fs-md t-strong">{s.label}</span>
              <span className="kbd">{s.keys}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
