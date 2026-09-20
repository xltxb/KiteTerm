import { Badge, Switch } from "@/components/ui";
import { useFeaturesStore } from "@/stores/features";
import { useSyncStore } from "@/stores/sync";
import type { SyncConfig } from "@/types";

const SCOPES: { key: keyof SyncConfig["scopes"]; label: string; desc: string; badge?: string }[] = [
  { key: "hosts", label: "主机配置", desc: "地址、端口、标签与分组" },
  { key: "keys", label: "密钥", desc: "私钥经主密码端到端加密后同步", badge: "E2E 加密" },
  { key: "snippets", label: "命令片段", desc: "Snippets 与使用统计" },
  { key: "settings", label: "主题与设置", desc: "外观、快捷键映射" },
  { key: "certs", label: "SSL 证书", desc: "证书、私钥与 ACME 账户密钥经主密码端到端加密后同步", badge: "E2E 加密" },
  { key: "recordings", label: "会话录制", desc: "asciicast 文件经主密码加密后上传;文件较大,建议按需开启" },
];

/** 同步范围:逐项开关决定哪些数据参与云同步 */
export function ScopeCard() {
  const { config, setScope } = useSyncStore();
  const flags = useFeaturesStore((s) => s.features);
  const blocked = (key: string) =>
    ((key === "keys" || key === "certs") && !flags.keySync) ||
    (key === "recordings" && !flags.recordingSync);

  return (
    <div className="card pad-band mb-16">
      {SCOPES.map((s) => (
        <div key={s.key} className="settings-row">
          <div className="grow lh-snug">
            <div className="row gap-8">
              <span className="w-semi fs-md t-strong">{s.label}</span>
              {s.badge && <Badge color="accent">{s.badge}</Badge>}
              {blocked(s.key) && <Badge color="warning">平台已禁止</Badge>}
            </div>
            <div className="fs-sm t-faint">{s.desc}</div>
          </div>
          <Switch
            on={(config?.scopes[s.key] ?? false) && !blocked(s.key)}
            onChange={(on) => !blocked(s.key) && void setScope(s.key, on)}
          />
        </div>
      ))}
    </div>
  );
}
