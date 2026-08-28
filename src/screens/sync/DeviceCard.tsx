import { Laptop, Monitor, Smartphone } from "lucide-react";
import { Badge } from "@/components/ui";
import { useSyncStore } from "@/stores/sync";

function deviceIcon(kind: string) {
  if (kind === "phone") return <Smartphone size={15} />;
  if (kind === "desktop") return <Monitor size={15} />;
  return <Laptop size={15} />;
}

/** 已注册设备列表:本机会额外打上标记 */
export function DeviceCard() {
  const status = useSyncStore((s) => s.status);

  return (
    <div className="card pad-band mb-16">
      {status.devices.length === 0 && (
        <div className="fs-sm t-faint py-16">暂无已注册设备。</div>
      )}
      {status.devices.map((d) => (
        <div key={d.id} className="settings-row">
          <div className="icon-chip md" data-tone="neutral">
            {deviceIcon(d.kind)}
          </div>
          <div className="grow lh-snug">
            <div className="w-semi fs-md t-strong">{d.name}</div>
            <div className="fs-xs t-faint">{d.current ? `本机 · ${d.lastSeenAt}` : d.lastSeenAt}</div>
          </div>
          {d.current && <Badge color="accent">本机</Badge>}
        </div>
      ))}
    </div>
  );
}
