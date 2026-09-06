import { useEffect, useState } from "react";
import { ShieldCheck } from "lucide-react";
import * as QRCode from "qrcode";
import { Badge, Button, TextInput } from "@/components/ui";
import { useFeaturesStore } from "@/stores/features";
import { useSyncStore } from "@/stores/sync";
import type { AccountInfo, TotpSetup } from "@/types";

/** 双因素认证:未绑定 → 生成密钥/二维码 → 输码确认;已绑定 → 输码解绑(平台强制时不可解绑) */
export function TotpCard({ account }: { account: AccountInfo }) {
  const { totpSetup, totpEnable, totpDisable } = useSyncStore();
  const enforced = useFeaturesStore((s) => s.features.totpRequired);
  const [setup, setSetup] = useState<TotpSetup | null>(null);
  const [qr, setQr] = useState<string>("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<string | null>(null);

  useEffect(() => {
    if (!setup) { setQr(""); return; }
    QRCode.toDataURL(setup.otpauthUrl, { margin: 1, width: 160 }).then(setQr).catch(() => setQr(""));
  }, [setup]);

  const run = async (fn: () => Promise<unknown>, ok: string) => {
    setBusy(true); setMsg(null);
    try { await fn(); setMsg(ok); setCode(""); }
    catch (e) { setMsg(String(e).replace(/^(?:Error:\s*)?[A-Z_]+:\s*/, "").replace(/\(HTTP \d+\)\s*$/, "")); }
    finally { setBusy(false); }
  };

  return (
    <div className="card pad-card mb-16">
      <div className="row gap-8 mb-4">
        <ShieldCheck size={15} />
        <span className="fs-md w-bold t-strong">双因素认证</span>
        <span className="spacer" />
        <Badge color={account.totpEnabled ? "success" : enforced ? "danger" : "neutral"} dot>
          {account.totpEnabled ? "已绑定" : enforced ? "平台强制,尚未绑定" : "未绑定"}
        </Badge>
      </div>
      {!account.totpEnabled && enforced && (
        <div className="alert alert-warning mb-12">平台要求所有账户绑定双因素认证,绑定前无法同步。</div>
      )}

      {account.totpEnabled ? (
        <div className="row gap-8">
          <span className="fs-sm t-muted grow">登录时需要验证器里的 6 位动态码。{enforced ? "平台强制开启,不能解绑。" : ""}</span>
          {!enforced && (
            <>
              <TextInput placeholder="动态码" value={code} inputMode="numeric" maxLength={6}
                onChange={(e) => setCode(e.target.value.replace(/\D/g, ""))} style={{ width: 120 }} />
              <Button variant="ghost" loading={busy} disabled={code.length !== 6}
                onClick={() => void run(() => totpDisable(code), "已解绑")}>解绑</Button>
            </>
          )}
        </div>
      ) : !setup ? (
        <Button variant="primary" loading={busy}
          onClick={() => void run(async () => setSetup(await totpSetup()), "用验证器扫码,然后输入动态码确认")}>
          开始绑定
        </Button>
      ) : (
        <div className="row gap-16" style={{ alignItems: "flex-start" }}>
          {qr && <img src={qr} alt="otpauth 二维码" width={160} height={160} className="qr" />}
          <div className="col gap-8 grow">
            <div className="fs-sm t-muted lh-relaxed">
              用 Google Authenticator / 1Password 等扫码,或手动输入密钥:
              <div className="mono fs-sm t-strong" style={{ wordBreak: "break-all" }}>{setup.secret}</div>
            </div>
            <div className="row gap-8">
              <TextInput placeholder="6 位动态码" value={code} inputMode="numeric" maxLength={6}
                onChange={(e) => setCode(e.target.value.replace(/\D/g, ""))} style={{ width: 140 }} />
              <Button variant="primary" loading={busy} disabled={code.length !== 6}
                onClick={() => void run(async () => { await totpEnable(code); setSetup(null); }, "已绑定双因素认证")}>确认绑定</Button>
              <Button variant="ghost" onClick={() => setSetup(null)}>取消</Button>
            </div>
          </div>
        </div>
      )}
      {msg && <div className="fs-sm t-muted mt-8">{msg}</div>}
    </div>
  );
}
