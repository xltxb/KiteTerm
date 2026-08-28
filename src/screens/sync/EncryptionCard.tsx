import { Lock } from "lucide-react";
import { Button } from "@/components/ui";
import { useSyncStore } from "@/stores/sync";

/** 端到端加密说明,以及恢复短语的查看/恢复入口 */
export function EncryptionCard({
  onRestore,
  onShowPhrase,
}: {
  onRestore: () => void;
  onShowPhrase: () => void;
}) {
  const config = useSyncStore((s) => s.config);

  return (
    <div className="card row gap-12 pad-card">
      <Lock size={15} />
      <div className="grow fs-sm t-muted lh-normal">
        同步数据以主密码派生密钥在本地加密,KiteTerm 与管理平台均无法读取你的私钥与主机凭据。
      </div>
      <Button variant="ghost" onClick={onRestore}>用恢复短语恢复</Button>
      <Button variant="outline" disabled={!config?.hasMasterKey} onClick={onShowPhrase}>
        查看恢复短语
      </Button>
    </div>
  );
}
