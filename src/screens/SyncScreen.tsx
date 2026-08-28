import { useEffect, useState } from "react";
import { Cloud, Laptop, Lock, Monitor, Smartphone, UserRound } from "lucide-react";
import { Badge, Button, ScreenHeader, Segmented, Switch, TextInput } from "@/components/ui";
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

/** 后端错误形如 `invite_required: 该平台需要邀请码才能注册(HTTP 400)`,展示时只留人话部分 */
function friendlyError(e: unknown): string {
  return String(e)
    .replace(/^(?:Error:\s*)?[A-Za-z_]+:\s*/, "")
    .replace(/[((]HTTP\s*\d+[))]\s*$/, "")
    .trim();
}

/** 口令强度评估:与服务端 password_issue 的规则保持一致,先在本地给出即时反馈 */
function checkPassword(pw: string): { ok: boolean; score: number; hint: string | null } {
  if (!pw) return { ok: false, score: 0, hint: null };
  const len = [...pw].length;
  const hasLetter = /\p{L}/u.test(pw);
  const hasDigit = /\p{N}/u.test(pw);
  const hasSymbol = /[^\p{L}\p{N}\s]/u.test(pw);
  const kinds = [hasLetter, hasDigit, hasSymbol].filter(Boolean).length;
  const weak = [
    "password", "password1", "password123", "12345678", "123456789", "1234567890",
    "qwerty123", "admin123", "abc12345", "iloveyou", "letmein1", "welcome1",
  ];
  if (len < 8) return { ok: false, score: Math.min(1, Math.floor(len / 4)), hint: "密码至少 8 位" };
  if (len > 128) return { ok: false, score: 1, hint: "密码最多 128 位" };
  if (kinds < 2) return { ok: false, score: 1, hint: "密码需包含字母、数字、符号中的至少两类" };
  if (new Set(pw).size < 4) return { ok: false, score: 1, hint: "密码字符过于单一" };
  if (weak.includes(pw.toLowerCase())) return { ok: false, score: 1, hint: "密码过于常见,请更换" };
  // 合格后按长度与字符种类给出 2~4 档强度
  let score = 2;
  if (len >= 12) score += 1;
  if (kinds === 3 && len >= 10) score += 1;
  return { ok: true, score: Math.min(4, score), hint: null };
}

/** 账户卡:同步由管理平台管控 —— 登录后由平台下发存储凭据并放开同步 */
function AccountCard() {
  const { account, config, login, register, logout, saveConfig } = useSyncStore();
  const [mode, setMode] = useState<"login" | "register">("login");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [invite, setInvite] = useState("");
  const [masterPw, setMasterPw] = useState("");
  const [busy, setBusy] = useState<"login" | "register" | "master" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [showPlatform, setShowPlatform] = useState(false);
  const [platformUrl, setPlatformUrl] = useState("");
  const [policy, setPolicy] = useState<{ mode: string; passwordMinLength: number; passwordRule: string } | null>(null);

  const loadPolicy = async () => {
    try {
      setPolicy(await ipc.accountRegisterPolicy());
    } catch {
      setPolicy(null); // 平台不可达时按开放注册渲染,真正的拦截以服务端为准
    }
  };

  useEffect(() => {
    void ipc.platformUrlGet().then(setPlatformUrl);
    void loadPolicy();
  }, []);

  const run = async (kind: "login" | "register", fn: () => Promise<void>) => {
    setBusy(kind);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(friendlyError(e));
    } finally {
      setBusy(null);
    }
  };

  if (!account) {
    const isRegister = mode === "register";
    const registerClosed = isRegister && policy?.mode === "closed";
    const needInvite = policy?.mode === "invite";
    const emailOk = /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim());
    const pwCheck = checkPassword(password);
    const canRegister =
      emailOk && pwCheck.ok && password === confirm && (!needInvite || invite.trim()) && !registerClosed;

    return (
      <div className="card" style={{ padding: "16px 20px", marginBottom: 16 }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 6 }}>
          <UserRound size={15} />
          <span style={{ fontSize: 13.5, fontWeight: 700, color: "var(--text-strong)" }}>
            {isRegister ? "注册管理平台账户" : "登录管理平台"}
          </span>
          <span style={{ flex: 1 }} />
          <Segmented
            options={[{ value: "login", label: "登录" }, { value: "register", label: "注册" }]}
            value={mode}
            onChange={(m) => { setMode(m); setError(null); }}
          />
        </div>
        <div style={{ fontSize: 12.5, color: "var(--text-muted)", lineHeight: 1.6, marginBottom: 14 }}>
          云同步由管理平台统一管控:注册并登录后,平台为你的账户下发专属存储空间并放开同步。主密码只在本地派生密钥,平台无法解密你的数据。
        </div>

        {registerClosed && (
          <div style={{ fontSize: 12.5, color: "var(--warning-text)", background: "var(--warning-subtle)", borderRadius: 10, padding: "10px 12px", marginBottom: 12, lineHeight: 1.6 }}>
            该平台当前未开放注册,请联系管理员开通账户。
          </div>
        )}

        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12 }}>
          <TextInput
            label="邮箱"
            placeholder="you@example.com"
            value={email}
            onChange={(e) => setEmail(e.target.value)}
          />
          <TextInput
            label={isRegister ? `密码(${policy?.passwordMinLength ?? 8} 位以上)` : "密码"}
            type="password"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !isRegister && email.trim() && password) {
                void run("login", () => login(email.trim(), password));
              }
            }}
          />
          {isRegister && (
            <TextInput
              label="确认密码"
              type="password"
              value={confirm}
              onChange={(e) => setConfirm(e.target.value)}
            />
          )}
          {isRegister && needInvite && (
            <TextInput
              label="邀请码"
              placeholder="XXXX-XXXX-XXXX"
              value={invite}
              onChange={(e) => setInvite(e.target.value.toUpperCase())}
            />
          )}
        </div>

        {isRegister && password.length > 0 && (
          <div style={{ marginTop: 10 }}>
            <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
              <div style={{ flex: 1, height: 4, borderRadius: 99, background: "var(--surface-sunken)", overflow: "hidden" }}>
                <div
                  style={{
                    height: "100%",
                    width: `${pwCheck.score * 25}%`,
                    borderRadius: 99,
                    background: pwCheck.score >= 3 ? "var(--success)" : pwCheck.score >= 2 ? "var(--warning)" : "var(--danger)",
                    transition: "width 220ms var(--ease-out)",
                  }}
                />
              </div>
              <span style={{ fontSize: 11, color: "var(--text-faint)", width: 52, textAlign: "right" }}>
                {["很弱", "弱", "一般", "较强", "很强"][pwCheck.score]}
              </span>
            </div>
            <div style={{ fontSize: 11.5, color: pwCheck.ok ? "var(--text-faint)" : "var(--warning-text)", marginTop: 6, lineHeight: 1.5 }}>
              {pwCheck.hint ?? policy?.passwordRule ?? "至少 8 位,且包含字母、数字、符号中的至少两类"}
            </div>
          </div>
        )}
        {isRegister && confirm.length > 0 && password !== confirm && (
          <div style={{ fontSize: 11.5, color: "var(--danger-text)", marginTop: 6 }}>两次输入的密码不一致</div>
        )}
        {isRegister && email.length > 0 && !emailOk && (
          <div style={{ fontSize: 11.5, color: "var(--danger-text)", marginTop: 6 }}>邮箱格式不正确</div>
        )}

        {showPlatform && (
          <div style={{ marginTop: 12 }}>
            <TextInput
              label="平台地址"
              value={platformUrl}
              onChange={(e) => setPlatformUrl(e.target.value)}
              onBlur={async () => {
                await ipc.platformUrlSet(platformUrl.trim());
                void loadPolicy();
              }}
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
          {isRegister ? (
            <Button
              variant="primary"
              loading={busy === "register"}
              disabled={!canRegister || busy !== null}
              onClick={() =>
                void run("register", () =>
                  register(email.trim(), password, needInvite ? invite.trim() : undefined),
                )
              }
            >
              注册并登录
            </Button>
          ) : (
            <Button
              variant="primary"
              loading={busy === "login"}
              disabled={!email.trim() || !password || busy !== null}
              onClick={() => void run("login", () => login(email.trim(), password))}
            >
              登录
            </Button>
          )}
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

function PhraseModal({ onClose }: { onClose: () => void }) {
  const [phrase, setPhrase] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    ipc.syncRecoveryPhrase().then(setPhrase).catch((e) => setError(String(e)));
  }, []);
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()} style={{ width: 560 }}>
        <h3 style={{ fontSize: 17, marginBottom: 8 }}>恢复短语</h3>
        <div style={{ fontSize: 12.5, color: "var(--warning-text)", lineHeight: 1.6, marginBottom: 14 }}>
          这 24 个单词等价于你的主密钥:请抄写到纸上妥善保管,任何人拿到即可解密你的同步数据。
          忘记主密码且无此短语 = 数据不可恢复。
        </div>
        {error && <div style={{ fontSize: 12.5, color: "var(--danger-text)" }}>{error}</div>}
        {phrase && (
          <div style={{ display: "grid", gridTemplateColumns: "repeat(4,1fr)", gap: 8 }}>
            {phrase.split(" ").map((w, i) => (
              <div key={i} style={{ display: "flex", alignItems: "center", gap: 6, background: "var(--surface-sunken)", border: "1px solid var(--border-subtle)", borderRadius: 8, padding: "6px 10px" }}>
                <span style={{ fontFamily: "var(--font-mono)", fontSize: 10, color: "var(--text-faint)", width: 16 }}>{i + 1}</span>
                <span style={{ fontFamily: "var(--font-mono)", fontSize: 12.5, color: "var(--text-strong)" }}>{w}</span>
              </div>
            ))}
          </div>
        )}
        <div style={{ display: "flex", justifyContent: "flex-end", marginTop: 18 }}>
          <Button variant="primary" onClick={onClose}>我已妥善保存</Button>
        </div>
      </div>
    </div>
  );
}

function RestoreModal({ onClose, onDone }: { onClose: () => void; onDone: () => void }) {
  const [phrase, setPhrase] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()} style={{ width: 520 }}>
        <h3 style={{ fontSize: 17, marginBottom: 8 }}>用恢复短语恢复主密钥</h3>
        <div style={{ fontSize: 12.5, color: "var(--text-muted)", lineHeight: 1.6, marginBottom: 12 }}>
          输入在旧设备上抄写的 24 个单词(空格分隔),即可在本机重建主密钥并解密同步数据。
        </div>
        <textarea
          value={phrase}
          onChange={(e) => setPhrase(e.target.value)}
          placeholder="word1 word2 word3 …"
          style={{ width: "100%", height: 90, resize: "vertical", background: "var(--surface-sunken)", border: "1px solid var(--border-default)", borderRadius: 10, padding: 10, fontFamily: "var(--font-mono)", fontSize: 12.5, color: "var(--text-strong)", outline: "none" }}
        />
        {error && <div style={{ fontSize: 12, color: "var(--danger-text)", marginTop: 8 }}>{error}</div>}
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 16 }}>
          <Button variant="ghost" onClick={onClose}>取消</Button>
          <Button
            variant="primary"
            loading={busy}
            disabled={phrase.trim().split(/\s+/).length !== 24}
            onClick={async () => {
              setBusy(true);
              setError(null);
              try {
                await ipc.syncRestoreMaster(phrase);
                onDone();
              } catch (e) {
                setError(String(e));
              } finally {
                setBusy(false);
              }
            }}
          >
            恢复
          </Button>
        </div>
      </div>
    </div>
  );
}

export function SyncScreen() {
  const { status, config, account, load, syncNow, setScope } = useSyncStore();
  const [showPhrase, setShowPhrase] = useState(false);
  const [showRestore, setShowRestore] = useState(false);

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
            同步数据以主密码派生密钥在本地加密,KiteTerm 与管理平台均无法读取你的私钥与主机凭据。
          </div>
          <Button variant="ghost" onClick={() => setShowRestore(true)}>用恢复短语恢复</Button>
          <Button variant="outline" disabled={!config?.hasMasterKey} onClick={() => setShowPhrase(true)}>
            查看恢复短语
          </Button>
        </div>
      </div>
      {showPhrase && <PhraseModal onClose={() => setShowPhrase(false)} />}
      {showRestore && (
        <RestoreModal
          onClose={() => setShowRestore(false)}
          onDone={() => { setShowRestore(false); void load(); }}
        />
      )}
    </div>
  );
}
