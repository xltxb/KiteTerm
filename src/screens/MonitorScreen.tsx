import { useEffect } from "react";
import { ScreenHeader } from "@/components/ui";
import { useMonitorStore } from "@/stores/monitor";

function Bar({ label, val }: { label: string; val: number }) {
  const color =
    val >= 90 ? "var(--danger)" :
    val >= 75 ? "var(--warning)" :
    "linear-gradient(90deg,var(--azure-500),var(--cyan-400))";
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
      <span style={{ fontFamily: "var(--font-mono)", fontSize: 10.5, letterSpacing: "0.06em", color: "var(--text-muted)", width: 34 }}>{label}</span>
      <div style={{ flex: 1, height: 6, borderRadius: 99, background: "var(--surface-sunken)", overflow: "hidden" }}>
        <div style={{ height: "100%", width: `${val}%`, borderRadius: 99, background: color, transition: "width 340ms var(--ease-out)" }} />
      </div>
      <span style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--text-strong)", width: 36, textAlign: "right" }}>{val}%</span>
    </div>
  );
}

export function MonitorScreen() {
  const { samples, load } = useMonitorStore();

  useEffect(() => { void load(); }, [load]);

  return (
    <div className="screen vscroll">
      <div className="screen-inner" style={{ maxWidth: 960 }}>
        <ScreenHeader eyebrow="MONITORING" title="服务器监控">
          <span style={{ fontSize: 12, color: "var(--text-faint)" }}>每 5s 刷新 · 经 SSH 采集,无需装 agent</span>
        </ScreenHeader>

        {samples.length === 0 && (
          <div className="card" style={{ padding: "36px 20px", textAlign: "center", color: "var(--text-faint)", fontSize: 13 }}>
            暂无监控数据 — 连接主机后自动开始采集。
          </div>
        )}

        <div style={{ display: "grid", gridTemplateColumns: "repeat(2,1fr)", gap: 14 }}>
          {samples.map((m) => (
            <div key={m.hostId} className="card hover-card" style={{ padding: "16px 18px", display: "flex", flexDirection: "column", gap: 12 }}>
              <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                <span style={{ width: 8, height: 8, borderRadius: 99, background: m.online ? "var(--success)" : "var(--danger)" }} />
                <span style={{ fontFamily: "var(--font-mono)", fontWeight: 600, fontSize: 13.5, color: "var(--text-strong)", flex: 1 }}>{m.hostName}</span>
                <span style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--text-faint)" }}>{m.uptime}</span>
              </div>
              <Bar label="CPU" val={m.cpu} />
              <Bar label="MEM" val={m.mem} />
              <Bar label="DISK" val={m.disk} />
              <div style={{ display: "flex", alignItems: "center", gap: 14, fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--text-muted)" }}>
                <span>{m.load}</span>
                <span>{m.netUp} {m.netDown}</span>
              </div>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}
