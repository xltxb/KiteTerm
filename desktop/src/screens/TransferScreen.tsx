import { useEffect } from "react";
import {
  ArrowLeft,
  ArrowRight,
  Download,
  File,
  FileArchive,
  FileCode,
  FileText,
  Folder,
  Laptop,
  Server,
  Upload,
} from "lucide-react";
import { Badge, Progress } from "@/components/ui";
import { ipc } from "@/ipc";
import { fmtBytes } from "@/lib/format";
import { useSessionStore } from "@/stores/session";
import { useTransferStore } from "@/stores/transfer";
import type { FileEntry, TransferJob } from "@/types";

function fileIcon(f: FileEntry) {
  if (f.isDir) return <Folder size={15} />;
  if (/\.(sh|js|ts|py|rb|go|rs)$/.test(f.name)) return <FileCode size={15} />;
  if (/\.(tar|gz|zip|xz|bz2|7z)/.test(f.name)) return <FileArchive size={15} />;
  if (/\.(log|txt|conf|env|md)/.test(f.name) || f.name.startsWith(".")) return <FileText size={15} />;
  return <File size={15} />;
}

/** 队列行右侧的状态文案与配色(tone 为文字颜色工具类) */
function jobMeta(j: TransferJob): { text: string; tone: string } {
  switch (j.state) {
    case "running":
      return { text: `${fmtBytes(j.rateBps, "—")}/s`, tone: "t-accent" };
    case "done":
      return { text: `完成 · ${(j.elapsedMs / 1000).toFixed(1)}s`, tone: "t-success" };
    case "failed":
      return { text: j.error ?? "失败", tone: "t-danger" };
    default:
      return { text: "排队中", tone: "t-faint" };
  }
}

function FilePane({
  title,
  icon,
  path,
  files,
  selected,
  onSelect,
  onOpenDir,
}: {
  title: string;
  icon: React.ReactNode;
  path: string;
  files: FileEntry[];
  selected: string | null;
  onSelect: (name: string | null) => void;
  onOpenDir: (name: string) => void;
}) {
  return (
    <div className="card col grow clip">
      <div className="row gap-8 pad-tight bd-bottom">
        {icon}
        <span className="w-bold fs-md t-strong">{title}</span>
        <span className="mono fs-xs t-faint ellipsis">{path}</span>
      </div>
      <div className="vscroll grow scroll">
        <div
          className="hover-row row gap-10 fs-md t-faint py-8 px-16 cursor-default"
          onDoubleClick={() => onOpenDir("..")}
        >
          <Folder size={15} /><span>..</span>
        </div>
        {files.map((f) => (
          <div
            key={f.name}
            className="hover-row row gap-10 fs-md py-8 px-16 cursor-default"
            onClick={() => onSelect(f.name === selected ? null : f.name)}
            onDoubleClick={() => f.isDir && onOpenDir(f.name)}
            style={{ background: f.name === selected ? "var(--accent-subtle)" : undefined }}
          >
            {fileIcon(f)}
            <span className="grow t-strong ellipsis">{f.name}</span>
            <span className="mono fs-xs t-faint text-right" style={{ width: 70 }}>
              {f.isDir ? "—" : fmtBytes(f.size, "—")}
            </span>
            <span className="fs-xs t-faint nowrap text-right" style={{ width: 96 }}>{f.modified}</span>
          </div>
        ))}
      </div>
    </div>
  );
}

function joinPath(base: string, name: string): string {
  if (name === "..") {
    const parts = base.replace(/\/+$/, "").split("/");
    parts.pop();
    const p = parts.join("/");
    return p || "/";
  }
  return base.replace(/\/+$/, "") + "/" + name;
}

export function TransferScreen() {
  const { sessions, activeId } = useSessionStore();
  const active = sessions.find((s) => s.id === activeId) ?? sessions[0] ?? null;
  const t = useTransferStore();

  useEffect(() => {
    void t.init(active?.id ?? null);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [active?.id]);

  const runningCount = t.jobs.filter((j) => j.state === "running").length;
  const queuedCount = t.jobs.filter((j) => j.state === "queued").length;

  return (
    <div className="col grow gap-16 clip py-24 px-28">
      <div className="row gap-12">
        <div>
          <div className="eyebrow">SFTP / SCP</div>
          <h1 className="screen-title fs-xl">文件传输</h1>
        </div>
        <span className="spacer" />
        {active ? (
          <Badge color="accent" dot>{active.hostName} 已连接</Badge>
        ) : (
          <Badge color="neutral">未连接 — 先从主机列表建立会话</Badge>
        )}
      </div>

      <div className="row grow min-h-0 astretch">
        <FilePane
          title="本地"
          icon={<Laptop size={15} />}
          path={t.localPath}
          files={t.localFiles}
          selected={t.localSel}
          onSelect={(n) => t.select("local", n)}
          onOpenDir={(n) => void t.setLocalPath(active?.id ?? null, joinPath(t.localPath, n))}
        />
        <div className="col fnone jcenter acenter gap-10" style={{ width: 56 }}>
          <div
            onClick={() => active && t.localSel && void t.upload(active.id)}
            title="上传所选"
            className="row jcenter r-full clickable"
            style={{
              width: 36, height: 36,
              background: active && t.localSel ? "var(--accent)" : "var(--surface-card)",
              border: active && t.localSel ? "none" : "1px solid var(--border-default)",
              color: active && t.localSel ? "var(--text-on-accent)" : "var(--text-faint)",
              boxShadow: active && t.localSel ? "var(--glow-sm)" : "none",
            }}
          >
            <ArrowRight size={16} />
          </div>
          <div
            onClick={() => active && t.remoteSel && void t.download(active.id)}
            title="下载所选"
            className="row jcenter r-full clickable"
            style={{
              width: 36, height: 36,
              background: active && t.remoteSel ? "var(--accent)" : "var(--surface-card)",
              border: active && t.remoteSel ? "none" : "1px solid var(--border-default)",
              color: active && t.remoteSel ? "var(--text-on-accent)" : "var(--text-muted)",
            }}
          >
            <ArrowLeft size={16} />
          </div>
        </div>
        <FilePane
          title={active?.hostName ?? "远端"}
          icon={<Server size={15} />}
          path={t.remotePath}
          files={t.remoteFiles}
          selected={t.remoteSel}
          onSelect={(n) => t.select("remote", n)}
          onOpenDir={(n) => void t.setRemotePath(active?.id ?? null, joinPath(t.remotePath, n))}
        />
      </div>

      <div className="card col fnone pad-card" style={{ maxHeight: 180 }}>
        <div className="row gap-8 mb-8">
          <span className="w-bold fs-md t-strong">传输队列</span>
          <span className="fs-xs t-faint">
            {runningCount} 进行中 · {queuedCount} 排队
          </span>
        </div>
        <div className="vscroll col gap-2 scroll">
          {t.jobs.length === 0 && <div className="fs-sm t-faint py-6 px-2">队列为空 — 选中文件后用中间箭头上传/下载。</div>}
          {t.jobs.map((j) => {
            const meta = jobMeta(j);
            return (
              <div key={j.id} className="row gap-12 py-8 px-2">
                {j.direction === "upload" ? <Upload size={15} /> : <Download size={15} />}
                <span className="mono fs-sm t-strong ellipsis" style={{ width: 210 }}>{j.name}</span>
                <span className="fs-xs t-faint grow ellipsis">
                  {j.direction === "upload" ? `${j.localPath} → ${j.remotePath}` : `${j.remotePath} → ${j.localPath}`}
                </span>
                <div style={{ width: 180 }}><Progress value={j.progress} /></div>
                <span className="mono fs-xs t-muted text-right" style={{ width: 48 }}>
                  {j.state === "queued" ? "—" : `${Math.round(j.progress)}%`}
                </span>
                <span className={`fs-xs ellipsis text-right ${meta.tone}`} style={{ width: 110 }} title={j.error ?? undefined}>{meta.text}</span>
                {j.state === "failed" && (
                  <span
                    onClick={() => void ipc.retryTransfer(j.id)}
                    className="fs-xs t-accent clickable nowrap"
                  >
                    重试
                  </span>
                )}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}
