import { useEffect, useState } from "react";
import { Cloud, Laptop, Lock, Monitor, Smartphone, UserRound } from "lucide-react";
import { Badge, Button, ScreenHeader, Switch, TextInput } from "@/components/ui";
import { ipc } from "@/ipc";
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

/** 账户卡:同步由管理平台管控 —— 登录后由平台下发存储凭据并放开同步 */
function AccountCard() {
  const { account, config, login, register, logout, saveConfig } = useSyncStore();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [masterPw, setMasterPw] = useState("");
  const [busy, setBusy] = useState<"login" | "register" | "master" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showPlatform, setShowPlatform] = useState(false);
  const [platformUrl, setPlatformUrl] = useState("");

  useEffect(() => {
    void ipc.platformUrlGet().then(setPlatformUrl);
  }, []);

  const run = async (kind: "login" | "register", fn: () => Promise<void>) => {
    setBusy(kind);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  if (!account) {
    return (
      <div className="card" style={{ padding: "16px 20px", marginBottom: 16 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 6 }}>
          <UserRound size={15} />
          <span style={{ fontSize: 13.5, fontWeight: 700, color: "var(--text-strong)" }}>登录管理平台</span>
        </div>
        <div style={{ fontSize: 12.5, color: "var(--text-muted)", lineHeight: 1.6, marginBottom: 14 }}>
          云同步由管理平台统一管控:注册并登录后,平台为你的账户下发专属存储空间并放开同步。主密码只在本地派生密钥,平台无法解密你的数据。
        </div>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12 }}>
          <TextInput label="邮箱" placeholder="you@example.com" value={email} onChange={(e) => setEmail(e.target.value)} />
          <TextInput label="密码(至少 8 位)" type="password" value={password} onChange={(e) => setPassword(e.target.value)} />
        </div>
        {showPlatform && (
          <div style={{ marginTop: 12 }}>
            <TextInput
              label="平台地址"
              value={platformUrl}
              onChange={(e) => setPlatformUrl(e.target.value)}
              onBlur={() => void ipc.platformUrlSet(platformUrl.trim())}
              style={{ width: "100%" }}
            />
          </div>
        )}
        {error && <div style={{ fontSize: 12, color: "var(--danger-text)", marginTop: 10, lineHeight: 1.5 }}>{error}</div>}
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 14 }}>
          <span
            onClick={() => setShowPlatform((s) => !s)}
            style={{ fontSize: 11.5, color: "var(--text-faint)", cursor: "pointer" }}
          >
            {showPlatform ? "收起平台设置" : "平台设置"}
          </span>
          <span style={{ flex: 1 }} />
          <Button
            variant="outline"
            loading={busy === "register"}
            disabled={!email.trim() || password.length < 8 || busy !== null}
            onClick={() => void run("register", () => register(email.trim(), password))}
          >
            注册并登录
          </Button>
          <Button
            variant="primary"
            loading={busy === "login"}
            disabled={!email.trim() || !password || busy !== null}
            onClick={() => void run("login", () => login(email.trim(), password))}
          >
            登录
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="card" style={{ padding: "14px 20px", marginBottom: 16, display: "flex", flexDirection: "column", gap: 12 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
        <div className="icon-chip" style={{ width: 34, height: 34 }}><UserRound size={16} /></div>
        <div style={{ flex: 1, lineHeight: 1.3 }}>
          <div style={{ fontWeight: 600, fontSize: 13.5, color: "var(--text-strong)" }}>{account.email}</div>
          <div style={{ fontSize: 11.5, color: "var(--text-faint)" }}>管理平台账户 · 存储由平台下发</div>
        </div>
        <Badge color={account.syncEnabled ? "success" : "danger"} dot>
          {account.syncEnabled ? "同步已放开" : "已被平台停用"}
        </Badge>
        <Button variant="ghost" onClick={() => void logout()}>退出登录</Button>
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: 10, borderTop: "1px solid var(--border-subtle)", paddingTop: 12 }}>
        <span style={{ fontSize: 12.5, fontWeight: 600, color: "var(--text-strong)", width: 64 }}>主密码</span>
        {config?.hasMasterKey && !masterPw ? (
          <>
            <span style={{ flex: 1, fontSize: 12, color: "var(--success-text)" }}>已设置 — 端到端加密就绪</span>
            <Button variant="ghost" onClick={() => setMasterPw(" ")}>修改</Button>
          </>
        ) : (
          <>
            <input
              className="input"
              type="password"
              placeholder="输入主密码(不上传,用于派生加密密钥)"
              value={masterPw.trim()}
              onChange={(e) => setMasterPw(e.target.value || " ")}
              style={{ flex: 1 }}
            />
            <Button
              variant="primary"
              loading={busy === "master"}
              disabled={masterPw.trim().length < 6}
              onClick={async () => {
                setBusy("master");
                try {
                  const cur = useSyncStore.getState().config;
                  await saveConfig({
                    endpoint: "", bucket: "", region: "", accessKey: "",
                    hasSecret: false, hasMasterKey: cur?.hasMasterKey ?? false,
                    scopes: cur?.scopes ?? { hosts: true, keys: true, snippets: true, settings: false },
                    masterPassword: masterPw.trim(),
                  });
                  setMasterPw("");
                } catch (e) {
                  setError(String(e));
                } finally {
                  setBusy(null);
                }
              }}
            >
              保存
            </Button>
          </>
        )}
      </div>
      {error && <div style={{ fontSize: 12, color: "var(--danger-text)", lineHeight: 1.5 }}>{error}</div>}
    </div>
  );
}

export function SyncScreen() {
  const { status, config, account, load, syncNow, setScope } = useSyncStore();

  useEffect(() => { void load(); }, [load]);

  const heroText = status.syncing
    ? "正在同步…"
    : !account
      ? "登录管理平台后开启云同步"
      : !account.syncEnabled
        ? "云同步已被平台停用"
        : !config?.hasMasterKey
          ? "设置主密码以启用端到端加密同步"
          : "云同步已开启";

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
              {heroText}
            </div>
            <div style={{ fontSize: 12.5, color: "var(--text-muted)" }}>
              上次同步 {status.lastSyncAt ?? "从未"} · {status.devices.length} 台设备一致 · 端到端加密
            </div>
          </div>
          <Button variant="primary" loading={status.syncing} disabled={!status.configured} onClick={() => void syncNow()}>
            立即同步
          </Button>
        </div>

        <AccountCard />

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
