import { useState } from "react";
import { Lock } from "lucide-react";
import { Button } from "@/components/ui";
import type { Host } from "@/types";

/** 目标主机未配置密钥时,回落到密码认证 */
export function PasswordModal({
  host,
  error,
  busy,
  onSubmit,
  onClose,
}: {
  host: Host;
  error: string | null;
  busy: boolean;
  onSubmit: (password: string) => void;
  onClose: () => void;
}) {
  const [pw, setPw] = useState("");
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal sm" onClick={(e) => e.stopPropagation()}>
        <h3 className="modal-title tight row gap-8">
          <Lock size={15} />密码认证
        </h3>
        <div className="fs-sm t-muted mb-16">
          {host.user}@{host.addr}:{host.port} 未配置密钥,请输入登录密码。
        </div>
        <input
          className="input w-full"
          type="password"
          autoFocus
          value={pw}
          onChange={(e) => setPw(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && pw && onSubmit(pw)}
          placeholder="密码"
        />
        {error && (
          <div className="fs-sm t-danger mt-12 lh-normal">{error}</div>
        )}
        <div className="modal-actions">
          <Button variant="ghost" onClick={onClose}>取消</Button>
          <Button variant="primary" disabled={!pw} loading={busy} onClick={() => onSubmit(pw)}>
            连接
          </Button>
        </div>
      </div>
    </div>
  );
}
