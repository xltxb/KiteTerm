import { useState } from "react";
import { MailCheck } from "lucide-react";
import { Button, TextInput } from "@/components/ui";
import { useSyncStore } from "@/stores/sync";

/** 平台开启邮箱验证且本账户未验证:同步会被拒(EMAIL_UNVERIFIED),在这里输码 */
export function EmailVerifyCard() {
  const { verifyEmail, resendVerification } = useSyncStore();
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState<"verify" | "resend" | null>(null);
  const [msg, setMsg] = useState<string | null>(null);

  const act = async (kind: "verify" | "resend", fn: () => Promise<unknown>, ok: string) => {
    setBusy(kind); setMsg(null);
    try { await fn(); setMsg(ok); }
    catch (e) { setMsg(String(e).replace(/^(?:Error:\s*)?[A-Z_]+:\s*/, "").replace(/\(HTTP \d+\)\s*$/, "")); }
    finally { setBusy(null); }
  };

  return (
    <div className="card pad-card mb-16">
      <div className="row gap-8 mb-4">
        <MailCheck size={15} />
        <span className="fs-md w-bold t-strong">验证邮箱</span>
      </div>
      <div className="fs-sm t-muted lh-relaxed mb-12">
        平台要求验证邮箱后才能开始同步。验证码已发送到你的邮箱(10 分钟内有效);若平台未配置邮件服务,请联系管理员获取或直接标记。
      </div>
      <div className="row gap-8">
        <TextInput placeholder="6 位验证码" value={code} inputMode="numeric" maxLength={6}
          onChange={(e) => setCode(e.target.value.replace(/\D/g, ""))} style={{ width: 160 }} />
        <Button variant="primary" loading={busy === "verify"} disabled={code.length !== 6 || busy !== null}
          onClick={() => void act("verify", () => verifyEmail(code), "邮箱已验证")}>验证</Button>
        <Button variant="ghost" loading={busy === "resend"} disabled={busy !== null}
          onClick={() => void act("resend", resendVerification, "已重新发送")}>重发验证码</Button>
      </div>
      {msg && <div className="fs-sm t-muted mt-8">{msg}</div>}
    </div>
  );
}
