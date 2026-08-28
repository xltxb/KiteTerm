import { Cloud } from "lucide-react";
import { Button } from "@/components/ui";
import { useSyncStore } from "@/stores/sync";

/** 顶部状态卡:一句话概括当前同步状态,并提供「立即同步」入口 */
export function SyncHeroCard() {
  const { status, config, account, syncNow } = useSyncStore();

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
    <div className="card row gap-16 p-20 mb-16">
      <div className="icon-chip lg glow">
        <Cloud size={22} />
      </div>
      <div className="grow lh-snug">
        <div className="w-bold fs-lg t-strong">{heroText}</div>
        <div className="fs-sm t-muted">
          上次同步 {status.lastSyncAt ?? "从未"} · {status.devices.length} 台设备一致 · 端到端加密
        </div>
      </div>
      <Button variant="primary" loading={status.syncing} disabled={!status.configured} onClick={() => void syncNow()}>
        立即同步
      </Button>
    </div>
  );
}
