import { useEffect, useState } from "react";
import { ChevronDown, Minus, Plus } from "lucide-react";
import { Button, ScreenHeader, Segmented, Switch, TextInput } from "@/components/ui";
import { ipc } from "@/ipc";
import { useFeaturesStore } from "@/stores/features";
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

/** 平台连接与功能开关。
 *
 *  这一段**永远显示**,不受任何开关影响 —— 功能开关的兜底是「从未拉到过就全关」,
 *  如果连平台地址都配不了,全新安装的用户就再也没法指向自己的平台、把功能打开。 */
function PlatformCard() {
  const { features, refresh } = useFeaturesStore();
  const [url, setUrl] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  useEffect(() => {
    void ipc.platformUrlGet().then(setUrl);
  }, []);

  const save = async () => {
    setBusy(true);
    setMsg(null);
    try {
      setUrl(await ipc.platformUrlSet(url));
      await refresh();
      const now = useFeaturesStore.getState().features;
      setMsg(
        now.known
          ? now.enforced
            ? "已连上平台,功能开关已更新"
            : "已连上平台。该平台未启用功能门禁,各项功能均可用"
          : "保存了,但没连上平台 —— 地址对吗?平台在跑吗?",
      );
    } catch (e) {
      setMsg(String(e).replace(/^[A-Za-z_]+:\s*/, ""));
    } finally {
      setBusy(false);
    }
  };

  const groups: { title: string; items: { label: string; on: boolean }[] }[] = [
    {
      title: "账户",
      items: [
        { label: "注册", on: features.registration !== "closed" },
        { label: "登录", on: features.login },
        { label: "邮箱验证", on: features.emailVerify },
        { label: "强制 2FA", on: features.totpRequired },
        { label: "多设备登录", on: features.multiDevice },
      ],
    },
    {
      title: "同步",
      items: [
        { label: "云同步", on: features.sync },
        { label: "密钥同步", on: features.keySync },
        { label: "录制同步", on: features.recordingSync },
      ],
    },
    {
      title: "AI",
      items: [
        { label: "AI 助手", on: features.ai },
        { label: "命令生成", on: features.ai && features.aiCommand },
        { label: "错误诊断", on: features.ai && features.aiDiagnose },
        { label: "输出摘要", on: features.ai && features.aiSummary },
      ],
    },
  ];

  return (
    <div className="card pad-band mb-16">
      <div className="settings-section-title">管理平台</div>

      <div className="settings-row">
        <div className="grow lh-snug">
          <div className="w-semi fs-md t-strong">平台地址</div>
          <div className="fs-sm t-faint">
            功能开关由管理平台下发。改完地址点「连接」即可重新拉取;公网地址必须走 https。
          </div>
        </div>
        <TextInput
          value={url}
          placeholder="https://m.kiteterm.com"
          onChange={(e) => setUrl(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && void save()}
          style={{ width: 240 }}
        />
        <Button variant="secondary" loading={busy} onClick={() => void save()}>
          连接
        </Button>
      </div>

      {msg && <div className="settings-row fs-sm t-muted lh-normal">{msg}</div>}

      <div className="settings-row">
        <div className="grow lh-snug">
          <div className="w-semi fs-md t-strong">功能开关</div>
          <div className="fs-sm t-faint">
            {!features.known
              ? "尚未连上管理平台 —— 按「未启用门禁」处理,各项功能均可用。"
              : features.enforced
                ? "该平台已启用功能门禁,以下为当前下发的状态,关闭的功能在本机隐藏入口。"
                : "该平台未启用功能门禁(部署配置 KITE_FEATURE_GATING),各项功能均可用。"}
          </div>
        </div>
      </div>

      {groups.map((g) => (
        <div key={g.title} className="settings-row">
          <div className="w-semi fs-md t-strong">{g.title}</div>
          <div className="row gap-8 wrap jend" style={{ maxWidth: 300 }}>
            {g.items.map((it) => (
              <span key={it.label} className={it.on ? "badge badge-success" : "badge badge-neutral"}>
                <span className="dot" />
                {it.label}
              </span>
            ))}
          </div>
        </div>
      ))}
    </div>
  );
}

export function SettingsScreen() {
  const { settings, setSettings } = useUiStore();
  const aiOn = useFeaturesStore((s) => s.features.ai);

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

        <PlatformCard />

        <div className="card pad-band mb-16">
          <div className="settings-section-title">行为</div>
          {BEHAVIORS.filter((b) => b.key !== "naturalLanguageInput" || aiOn).map((b) => (
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
          {SHORTCUTS.filter((s) => aiOn || !s.label.startsWith("AI")).map((s) => (
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
