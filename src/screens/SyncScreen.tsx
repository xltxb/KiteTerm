import { useEffect, useState } from "react";
import { Cloud, Laptop, Lock, Monitor, Smartphone } from "lucide-react";
import { Badge, Button, ScreenHeader, Switch, TextInput } from "@/components/ui";
import { useSyncStore } from "@/stores/sync";
import type { SyncConfig } from "@/types";

const SCOPES: { key: keyof SyncConfig["scopes"]; label: string; desc: string; badge?: string }[] = [
  { key: "hosts", label: "主机配置", desc: "地址、端口、标签与分组" },
  { key: "keys", label: "密钥", desc: "私钥经主密码端到端加密后同步", badge: "E2E 加密" },
  { key: "snippets", label: "命令片段", desc: "Snippets 与使用统计" },
  { key: "settings", label: "主题与设置", desc: "外观、快捷键映射" },
];

function deviceIcon(kind: string) {
  if (kind === "phone") return <Smartphone size={15} />;
  if (kind === "desktop") return <Monitor size={15} />;
  return <Laptop size={15} />;
}

function ConfigCard() {
  const { config, saveConfig } = useSyncStore();
  const [editing, setEditing] = useState(false);
  const [form, setForm] = useState({ endpoint: "", bucket: "", region: "auto", accessKey: "", secretKey: "", masterPassword: "" });
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    if (config) {
      setForm((f) => ({ ...f, endpoint: config.endpoint, bucket: config.bucket, region: config.region, accessKey: config.accessKey }));
    }
  }, [config]);

  if (!editing) {
    return (
      <div className="card" style={{ display: "flex", alignItems: "center", gap: 12, padding: "14px 20px", marginBottom: 16 }}>
        <Cloud size={15} />
        <div style={{ flex: 1, fontSize: 12.5, color: "var(--text-muted)", lineHeight: 1.5 }}>
          {config?.endpoint
            ? <>存储后端:<span style={{ fontFamily: "var(--font-mono)", color: "var(--text-body)" }}>{config.endpoint}/{config.bucket}</span>(S3 兼容,仅存密文)</>
            : "尚未配置同步存储 — 填入你自己的 S3 兼容桶(AWS S3 / R2 / MinIO)。"}
        </div>
        <Button variant="outline" onClick={() => setEditing(true)}>{config?.endpoint ? "修改配置" : "配置存储"}</Button>
      </div>
    );
  }

  return (
    <div className="card" style={{ padding: "16px 20px", marginBottom: 16 }}>
      <div style={{ fontSize: 12, fontWeight: 700, color: "var(--text-muted)", marginBottom: 12 }}>S3 兼容存储配置</div>
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12 }}>
        <TextInput label="Endpoint" placeholder="https://<account>.r2.cloudflarestorage.com" value={form.endpoint} onChange={(e) => setForm({ ...form, endpoint: e.target.value })} />
        <TextInput label="Bucket" placeholder="vela-sync" value={form.bucket} onChange={(e) => setForm({ ...form, bucket: e.target.value })} />
        <TextInput label="Region" placeholder="auto" value={form.region} onChange={(e) => setForm({ ...form, region: e.target.value })} />
        <TextInput label="Access Key" value={form.accessKey} onChange={(e) => setForm({ ...form, accessKey: e.target.value })} />
        <TextInput label="Secret Key" type="password" placeholder={useSyncStore.getState().config?.hasSecret ? "(保持不变)" : ""} value={form.secretKey} onChange={(e) => setForm({ ...form, secretKey: e.target.value })} />
        <TextInput label="主密码(派生加密密钥,不上传)" type="password" value={form.masterPassword} onChange={(e) => setForm({ ...form, masterPassword: e.target.value })} />
      </div>
      <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 16 }}>
        <Button variant="ghost" onClick={() => setEditing(false)}>取消</Button>
        <Button
          variant="primary"
          loading={busy}
          disabled={!form.endpoint || !form.bucket}
          onClick={async () => {
            setBusy(true);
            try {
              const cur = useSyncStore.getState().config;
              await saveConfig({
                endpoint: form.endpoint, bucket: form.bucket, region: form.region,
                accessKey: form.accessKey, hasSecret: cur?.hasSecret ?? false,
                scopes: cur?.scopes ?? { hosts: true, keys: true, snippets: true, settings: false },
                secretKey: form.secretKey || undefined,
                masterPassword: form.masterPassword || undefined,
              });
              setEditing(false);
            } finally { setBusy(false); }
          }}
        >
          保存
        </Button>
      </div>
    </div>
  );
}

export function SyncScreen() {
  const { status, config, load, syncNow, setScope } = useSyncStore();

  useEffect(() => { void load(); }, [load]);

  return (
    <div className="screen vscroll">
      <div className="screen-inner" style={{ maxWidth: 760 }}>
        <ScreenHeader eyebrow="CLOUD SYNC" title="云同步" />

        <div className="card" style={{ padding: "20px 22px", display: "flex", alignItems: "center", gap: 16, marginBottom: 16 }}>
          <div className="icon-chip" style={{ width: 46, height: 46, borderRadius: 14, boxShadow: "var(--glow-sm)" }}>
            <Cloud size={22} />
          </div>
          <div style={{ flex: 1, lineHeight: 1.4 }}>
            <div style={{ fontWeight: 700, fontSize: 15, color: "var(--text-strong)" }}>
              {status.syncing ? "正在同步…" : status.configured ? "云同步已开启" : "云同步未配置"}
            </div>
            <div style={{ fontSize: 12.5, color: "var(--text-muted)" }}>
              上次同步 {status.lastSyncAt ?? "从未"} · {status.devices.length} 台设备一致 · 端到端加密
            </div>
          </div>
          <Button variant="primary" loading={status.syncing} disabled={!status.configured} onClick={() => void syncNow()}>
            立即同步
          </Button>
        </div>

        <ConfigCard />

        <div className="card" style={{ padding: "8px 20px", marginBottom: 16 }}>
          {SCOPES.map((s) => (
            <div key={s.key} className="settings-row">
              <div style={{ flex: 1, lineHeight: 1.35 }}>
                <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                  <span style={{ fontWeight: 600, fontSize: 13.5, color: "var(--text-strong)" }}>{s.label}</span>
                  {s.badge && <Badge color="accent">{s.badge}</Badge>}
                </div>
                <div style={{ fontSize: 12, color: "var(--text-faint)" }}>{s.desc}</div>
              </div>
              <Switch on={config?.scopes[s.key] ?? false} onChange={(on) => void setScope(s.key, on)} />
            </div>
          ))}
        </div>

        <div className="card" style={{ padding: "8px 20px", marginBottom: 16 }}>
          {status.devices.length === 0 && (
            <div style={{ padding: "16px 0", fontSize: 12.5, color: "var(--text-faint)" }}>暂无已注册设备。</div>
          )}
          {status.devices.map((d) => (
            <div key={d.id} className="settings-row">
              <div style={{ width: 32, height: 32, flex: "none", borderRadius: 10, background: "var(--surface-sunken)", color: "var(--text-muted)", display: "flex", alignItems: "center", justifyContent: "center" }}>
                {deviceIcon(d.kind)}
              </div>
              <div style={{ flex: 1, lineHeight: 1.3 }}>
                <div style={{ fontWeight: 600, fontSize: 13, color: "var(--text-strong)" }}>{d.name}</div>
                <div style={{ fontSize: 11.5, color: "var(--text-faint)" }}>{d.current ? `本机 · ${d.lastSeenAt}` : d.lastSeenAt}</div>
              </div>
              {d.current && <Badge color="accent">本机</Badge>}
            </div>
          ))}
        </div>

        <div className="card" style={{ display: "flex", alignItems: "center", gap: 12, padding: "14px 20px" }}>
          <Lock size={15} />
          <div style={{ flex: 1, fontSize: 12.5, color: "var(--text-muted)", lineHeight: 1.5 }}>
            同步数据以主密码派生密钥在本地加密,Vela 无法读取你的私钥与主机凭据。
          </div>
          <Button variant="outline">查看恢复短语</Button>
        </div>
      </div>
    </div>
  );
}
