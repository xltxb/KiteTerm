import { useEffect, useState } from "react";
import { UserRound } from "lucide-react";
import { Button, Segmented, TextInput } from "@/components/ui";
import { ipc } from "@/ipc";
import { AccountSummary } from "@/screens/sync/AccountSummary";
import { checkPassword, PasswordStrength } from "@/screens/sync/PasswordStrength";
import { useSyncStore } from "@/stores/sync";

const DEFAULT_PW_RULE = "至少 8 位,且包含字母、数字、符号中的至少两类";

/** 后端错误形如 `invite_required: 该平台需要邀请码才能注册(HTTP 400)`,展示时只留人话部分 */
function friendlyError(e: unknown): string {
  return String(e)
    .replace(/^(?:Error:\s*)?[A-Za-z_]+:\s*/, "")
    .replace(/[((]HTTP\s*\d+[))]\s*$/, "")
    .trim();
}

/** 账户卡:同步由管理平台管控 —— 登录后由平台下发存储凭据并放开同步 */
export function AccountCard() {
  const { account, login, register } = useSyncStore();
  const [mode, setMode] = useState<"login" | "register">("login");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [invite, setInvite] = useState("");
  const [busy, setBusy] = useState<"login" | "register" | null>(null);
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

  if (account) return <AccountSummary account={account} />;

  const isRegister = mode === "register";
  const registerClosed = isRegister && policy?.mode === "closed";
  const needInvite = policy?.mode === "invite";
  const emailOk = /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email.trim());
  const pwCheck = checkPassword(password);
  const canRegister =
    emailOk && pwCheck.ok && password === confirm && (!needInvite || invite.trim()) && !registerClosed;

  return (
    <div className="card pad-card mb-16">
      <div className="row gap-8 mb-4">
        <UserRound size={15} />
        <span className="fs-md w-bold t-strong">
          {isRegister ? "注册管理平台账户" : "登录管理平台"}
        </span>
        <span className="spacer" />
        <Segmented
          options={[{ value: "login", label: "登录" }, { value: "register", label: "注册" }]}
          value={mode}
          onChange={(m) => { setMode(m); setError(null); }}
        />
      </div>
      <div className="fs-sm t-muted lh-relaxed mb-12">
        云同步由管理平台统一管控:注册并登录后,平台为你的账户下发专属存储空间并放开同步。主密码只在本地派生密钥,平台无法解密你的数据。
      </div>

      {registerClosed && (
        <div className="alert alert-warning mb-12">
          该平台当前未开放注册,请联系管理员开通账户。
        </div>
      )}

      <div className="form-grid">
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
        <PasswordStrength check={pwCheck} rule={policy?.passwordRule ?? DEFAULT_PW_RULE} />
      )}
      {isRegister && confirm.length > 0 && password !== confirm && (
        <div className="fs-xs t-danger mt-4">两次输入的密码不一致</div>
      )}
      {isRegister && email.length > 0 && !emailOk && (
        <div className="fs-xs t-danger mt-4">邮箱格式不正确</div>
      )}

      {showPlatform && (
        <div className="mt-12">
          <TextInput
            label="平台地址"
            value={platformUrl}
            onChange={(e) => setPlatformUrl(e.target.value)}
            onBlur={async () => {
              await ipc.platformUrlSet(platformUrl.trim());
              void loadPolicy();
            }}
          />
        </div>
      )}
      {error && <div className="fs-sm t-danger mt-8 lh-normal">{error}</div>}
      <div className="row gap-8 mt-12">
        <span className="fs-xs t-faint clickable" onClick={() => setShowPlatform((s) => !s)}>
          {showPlatform ? "收起平台设置" : "平台设置"}
        </span>
        <span className="spacer" />
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
