import { useState } from "react";
import { UserRound } from "lucide-react";
import { Badge, Button } from "@/components/ui";
import { useFeaturesStore } from "@/stores/features";
import { useSyncStore } from "@/stores/sync";
import type { AccountInfo } from "@/types";
import { EmailVerifyCard } from "@/screens/sync/EmailVerifyCard";
import { TotpCard } from "@/screens/sync/TotpCard";

/** 已登录状态的账户卡:账户信息、平台放行状态,以及本地主密码的设置入口;
 *  邮箱未验证/2FA 未绑定时,再叠加对应的引导卡片 */
export function AccountSummary({ account }: { account: AccountInfo }) {
  const { config, logout, saveConfig } = useSyncStore();
  const emailVerify = useFeaturesStore((s) => s.features.emailVerify);
  const [masterPw, setMasterPw] = useState("");
  const [busy, setBusy] = useState<"master" | null>(null);
  const [error, setError] = useState<string | null>(null);

  return (
    <>
      <div className="card col gap-12 pad-card mb-16">
        <div className="row gap-12">
          <div className="icon-chip md"><UserRound size={16} /></div>
          <div className="grow lh-snug">
            <div className="w-semi fs-md t-strong">{account.email}</div>
            <div className="fs-xs t-faint">管理平台账户 · 存储由平台下发</div>
          </div>
          {!account.emailVerified && <Badge color="warning" dot>邮箱未验证</Badge>}
          <Badge color={account.syncEnabled ? "success" : "danger"} dot>
            {account.syncEnabled ? "同步已放开" : "已被平台停用"}
          </Badge>
          <Button variant="ghost" onClick={() => void logout()}>退出登录</Button>
        </div>
        <div className="row gap-10 bd-top pt-12">
          <span className="fs-sm w-semi t-strong label-col">主密码</span>
          {config?.hasMasterKey && !masterPw ? (
            <>
              <span className="spacer fs-sm t-success">已设置 — 端到端加密就绪</span>
              <Button variant="ghost" onClick={() => setMasterPw(" ")}>修改</Button>
            </>
          ) : (
            <>
              <input
                className="input spacer"
                type="password"
                placeholder="输入主密码(不上传,用于派生加密密钥)"
                value={masterPw.trim()}
                onChange={(e) => setMasterPw(e.target.value || " ")}
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
                      scopes: cur?.scopes ?? { hosts: true, keys: true, snippets: true, settings: false, recordings: false },
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
        {error && <div className="fs-sm t-danger lh-normal">{error}</div>}
      </div>
      {emailVerify && !account.emailVerified && <EmailVerifyCard />}
      <TotpCard account={account} />
    </>
  );
}
