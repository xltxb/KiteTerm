import { useState } from "react";
import { ArrowRightLeft, Trash2 } from "lucide-react";
import { Button, ScreenHeader, Segmented, Switch, TextInput } from "@/components/ui";
import { useDataStore } from "@/stores/data";
import type { ForwardRule, ForwardType } from "@/types";

function fmtBytes(n: number): string {
  if (n <= 0) return "0 B";
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(1)} GB`;
}

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
      <div className="screen-inner" style={{ maxWidth: 880 }}>
        <ScreenHeader eyebrow="PORT FORWARDING" title="端口转发" count={`${activeCount} 活跃`} />

        <div className="card" style={{ padding: "16px 20px", marginBottom: 16 }}>
          <div style={{ fontSize: 12, fontWeight: 700, color: "var(--text-muted)", marginBottom: 12 }}>新建规则</div>
          <div style={{ display: "flex", alignItems: "flex-end", gap: 10, flexWrap: "wrap" }}>
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
              <select className="input" style={{ width: "100%" }} value={via} onChange={(e) => setVia(e.target.value)}>
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
          <div style={{ fontSize: 11.5, color: "var(--text-faint)", marginTop: 10 }}>
            L 本地转发 · R 远程转发 · D 动态 SOCKS5 代理
          </div>
        </div>

        <div className="card" style={{ padding: "6px 20px" }}>
          {forwards.length === 0 && (
            <div style={{ padding: "24px 0", textAlign: "center", fontSize: 13, color: "var(--text-faint)" }}>还没有转发规则</div>
          )}
          {forwards.map((f) => (
            <div key={f.id} style={{ display: "flex", alignItems: "center", gap: 14, padding: "13px 0", borderBottom: "1px solid var(--border-subtle)" }}>
              <span style={{ width: 28, height: 28, flex: "none", borderRadius: 8, display: "flex", alignItems: "center", justifyContent: "center", fontFamily: "var(--font-mono)", fontSize: 11, fontWeight: 700, background: "var(--accent-subtle)", color: "var(--accent-text)" }}>
                {f.type}
              </span>
              <div style={{ flex: 1, minWidth: 0, lineHeight: 1.4 }}>
                <div style={{ fontFamily: "var(--font-mono)", fontSize: 12.5, color: "var(--text-strong)" }}>{ruleText(f)}</div>
                <div style={{ fontSize: 11.5, color: "var(--text-faint)" }}>
                  经 {hostName(f.viaHostId)} · ↑ {fmtBytes(f.bytesUp)} · ↓ {fmtBytes(f.bytesDown)}
                </div>
              </div>
              <span style={{ display: "flex", alignItems: "center", gap: 6, fontSize: 11.5, color: f.enabled ? "var(--success-text)" : "var(--text-faint)" }}>
                <span style={{ width: 6, height: 6, borderRadius: 99, background: f.enabled ? "var(--success-text)" : "var(--text-faint)" }} />
                {f.enabled ? "活跃" : "已停用"}
              </span>
              <Switch on={f.enabled} onChange={(on) => void toggleForward(f.id, on)} />
              <div
                className="hover-row"
                onClick={() => { if (confirm("删除该转发规则?")) void deleteForward(f.id); }}
                style={{ padding: 6, borderRadius: 8, cursor: "pointer", color: "var(--text-faint)" }}
              >
                <Trash2 size={15} />
              </div>
            </div>
          ))}
        </div>

        <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 14, fontSize: 12, color: "var(--text-faint)" }}>
          <ArrowRightLeft size={13} />
          规则随主机配置一并云同步;断线重连后自动恢复活跃规则。
        </div>
      </div>
    </div>
  );
}
