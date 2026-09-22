import { useEffect } from "react";
import { Readout, ScreenHeader } from "@/components/ui";
import { useMonitorStore } from "@/stores/monitor";

function Bar({ label, val }: { label: string; val: number }) {
  const tone: "ok" | "warn" | "bad" = val >= 90 ? "bad" : val >= 75 ? "warn" : "ok";
  const color =
    val >= 90 ? "var(--danger)" :
    val >= 75 ? "var(--warning)" :
    "var(--accent)";
  return (
    <div className="row gap-10">
      <div style={{ width: 64 }}>
        <Readout label={label} value={val} unit="%" tone={tone} />
      </div>
      {/* 填充宽度与配色随读数变化,只有这两项保留内联 */}
      <div className="progress grow">
        <div style={{ width: `${val}%`, background: color }} />
      </div>
    </div>
  );
}

export function MonitorScreen() {
  const { samples, load } = useMonitorStore();

  useEffect(() => { void load(); }, [load]);

  return (
    <div className="screen vscroll">
      <div className="screen-inner wide">
        <ScreenHeader eyebrow="MONITORING" title="服务器监控" screen="monitor">
          <span className="fs-sm t-faint">每 5s 刷新 · 经 SSH 采集,无需装 agent</span>
        </ScreenHeader>

        {samples.length === 0 && (
          <div className="card empty-state fs-md" style={{ padding: "36px 20px" }}>
            暂无监控数据 — 连接主机后自动开始采集。
          </div>
        )}

        <div className="grid-2">
          {samples.map((m) => (
            <div key={m.hostId} className="card hover-card col gap-12 pad-card">
              <div className="row gap-10">
                <span className={m.online ? "dot-lg dot-ok" : "dot-lg dot-danger"} />
                <span className="grow mono w-semi fs-md t-strong">{m.hostName}</span>
                <span className="mono fs-xs t-faint">{m.uptime}</span>
              </div>
              <Bar label="CPU" val={m.cpu} />
              <Bar label="MEM" val={m.mem} />
              <Bar label="DISK" val={m.disk} />
              <div className="row gap-14 mono fs-xs t-muted">
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
