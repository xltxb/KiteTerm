import { useState } from "react";
import { ArrowRightLeft, Trash2 } from "lucide-react";
import { Button, ScreenHeader, Segmented, Switch, TextInput } from "@/components/ui";
import { fmtBytes } from "@/lib/format";
import { useDataStore } from "@/stores/data";
import type { ForwardRule, ForwardType } from "@/types";
import { confirmAction } from "@/lib/dialog";

function ruleText(f: ForwardRule): string {
  if (f.type === "D") return `SOCKS5 · localhost:${f.localPort}`;
  if (f.type === "R") return `远端:${f.localPort} → ${f.target}`;
  return `localhost:${f.localPort} → ${f.target}`;
}

export function ForwardsScreen() {
  const { forwards, hosts, createForward, toggleForward, deleteForward } = useDataStore();
  const [type, setType] = useState<ForwardType>("L");
  const [localPort, setLocalPort] = useState("");
  const [target, setTarget] = useState("");
  const [via, setVia] = useState("");

  const activeCount = forwards.filter((f) => f.enabled).length;
  const hostName = (id: string) => hosts.find((h) => h.id === id)?.name ?? id;
  const valid = Number(localPort) > 0 && (type === "D" || target.includes(":")) && via;

  return (
    <div className="screen vscroll">
      <div className="screen-inner medium">
        <ScreenHeader eyebrow="PORT FORWARDING" title="端口转发" count={`${activeCount} 活跃`} />

        <div className="card pad-card mb-16">
          <div className="fs-sm w-bold t-muted mb-12">新建规则</div>
          {/* 表单一行排布,控件底对齐(label 高度不同) */}
          <div className="row wrap aend gap-10">
            <Segmented
              mono
              options={[{ value: "L", label: "L" }, { value: "R", label: "R" }, { value: "D", label: "D" }]}
              value={type}
              onChange={setType}
            />
            <TextInput label="本地端口" placeholder="8080" value={localPort} onChange={(e) => setLocalPort(e.target.value)} style={{ width: 110 }} />
            {type !== "D" && (
              <TextInput label="目标 host:port" placeholder="db-primary:5432" value={target} onChange={(e) => setTarget(e.target.value)} style={{ width: 190 }} />
            )}
            <div style={{ width: 160 }}>
              <label className="input-label">经由主机</label>
              <select className="input w-full" value={via} onChange={(e) => setVia(e.target.value)}>
                <option value="">选择主机…</option>
                {hosts.map((h) => <option key={h.id} value={h.id}>{h.name}</option>)}
              </select>
            </div>
            <Button
              variant="primary"
              disabled={!valid}
              onClick={async () => {
                await createForward({ type, localPort: Number(localPort), target: type === "D" ? "" : target, viaHostId: via });
                setLocalPort(""); setTarget("");
              }}
            >
              创建
            </Button>
          </div>
          <div className="fs-xs t-faint mt-8">
            L 本地转发 · R 远程转发 · D 动态 SOCKS5 代理
          </div>
        </div>

        <div className="card">
          {forwards.length === 0 && (
            <div className="empty-state fs-md" style={{ padding: "24px 0" }}>还没有转发规则</div>
          )}
          {forwards.map((f) => (
            <div key={f.id} className="list-row">
              <span className="icon-chip sm mono fs-xs w-bold">
                {f.type}
              </span>
              <div className="grow lh-normal">
                <div className="mono fs-sm t-strong">{ruleText(f)}</div>
                <div className="fs-xs t-faint">
                  经 {hostName(f.viaHostId)} · ↑ {fmtBytes(f.bytesUp)} · ↓ {fmtBytes(f.bytesDown)}
                </div>
              </div>
              <span className={f.enabled ? "row gap-6 fs-xs t-success" : "row gap-6 fs-xs t-faint"}>
                <span className={f.enabled ? "dot-sm dot-ok" : "dot-sm"} />
                {f.enabled ? "活跃" : "已停用"}
              </span>
              <Switch on={f.enabled} onChange={(on) => void toggleForward(f.id, on)} />
              <div
                className="icon-btn sm plain"
                onClick={async () => { if (await confirmAction("删除该转发规则?", "删除转发规则")) await deleteForward(f.id); }}
              >
                <Trash2 size={15} />
              </div>
            </div>
          ))}
        </div>

        <div className="row gap-8 fs-sm t-faint mt-12">
          <ArrowRightLeft size={13} />
          规则随主机配置一并云同步;断线重连后自动恢复活跃规则。
        </div>
      </div>
    </div>
  );
}
