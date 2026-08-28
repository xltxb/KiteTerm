import { Cpu, Database, Globe, KeyRound, Pencil, Server, Trash2 } from "lucide-react";
import { Button } from "@/components/ui";
import type { Host } from "@/types";

/** 按名称/标签猜一个更贴切的主机图标 */
function hostIcon(h: Host) {
  const tags = h.tags.join(" ");
  if (/postgres|redis|mysql|db/.test(tags) || h.name.includes("db") || h.name.includes("redis")) return <Database size={17} />;
  if (h.name.includes("pi")) return <Cpu size={17} />;
  if (h.name.includes("vps")) return <Globe size={17} />;
  return <Server size={17} />;
}

/** 延迟档位对应的状态点颜色 */
function latColor(ms: number | null) {
  if (ms == null) return "var(--text-faint)";
  if (ms < 50) return "var(--success)";
  if (ms < 120) return "var(--warning)";
  return "var(--danger)";
}

/** 主机列表里的一行:身份 / 标签 / 延迟 / 密钥 / 操作 */
export function HostRow({
  host,
  keyName,
  connecting,
  onConnect,
  onEdit,
  onDelete,
}: {
  host: Host;
  keyName: string;
  connecting: boolean;
  onConnect: () => void;
  onEdit: () => void;
  onDelete: () => void;
}) {
  return (
    <div className="list-row hoverable">
      <div className="icon-chip md">{hostIcon(host)}</div>
      <div className="fnone lh-snug" style={{ width: 200 }}>
        <div className="fs-md w-semi t-strong">{host.name}</div>
        <div className="mono fs-xs t-faint">
          {host.user}@{host.addr} · :{host.port}
        </div>
      </div>
      <div className="row gap-6 grow clip">
        {host.tags.map((t) => <span key={t} className="tag-pill">{t}</span>)}
      </div>
      <span className="row gap-6 mono fs-xs t-muted" style={{ width: 64 }}>
        <span className="dot-sm" style={{ background: latColor(host.latencyMs) }} />
        {host.latencyMs != null ? `${host.latencyMs}ms` : "—"}
      </span>
      <span className="row gap-6 fs-xs t-faint" style={{ width: 150 }}>
        <KeyRound size={12} />{keyName}
      </span>
      <span className="fs-xs t-faint" style={{ width: 96 }}>{host.lastConnectedAt ?? "从未连接"}</span>
      <Button variant="secondary" loading={connecting} onClick={onConnect}>连接</Button>
      <div className="icon-btn sm plain" onClick={onEdit} title="编辑">
        <Pencil size={15} />
      </div>
      <div className="icon-btn sm plain" onClick={onDelete}>
        <Trash2 size={15} />
      </div>
    </div>
  );
}
