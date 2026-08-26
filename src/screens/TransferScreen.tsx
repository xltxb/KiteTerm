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
import { useSessionStore } from "@/stores/session";
import { useTransferStore } from "@/stores/transfer";
import type { FileEntry, TransferJob } from "@/types";

function fmtSize(n: number): string {
  if (n <= 0) return "—";
  if (n < 1024) return `${n} B`;
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 ** 3) return `${(n / 1024 ** 2).toFixed(1)} MB`;
  return `${(n / 1024 ** 3).toFixed(1)} GB`;
}

function fileIcon(f: FileEntry) {
  if (f.isDir) return <Folder size={15} />;
  if (/\.(sh|js|ts|py|rb|go|rs)$/.test(f.name)) return <FileCode size={15} />;
  if (/\.(tar|gz|zip|xz|bz2|7z)/.test(f.name)) return <FileArchive size={15} />;
  if (/\.(log|txt|conf|env|md)/.test(f.name) || f.name.startsWith(".")) return <FileText size={15} />;
  return <File size={15} />;
}

function jobMeta(j: TransferJob): { text: string; color: string } {
  switch (j.state) {
    case "running":
      return { text: `${fmtSize(j.rateBps)}/s`, color: "var(--accent-text)" };
    case "done":
      return { text: `完成 · ${(j.elapsedMs / 1000).toFixed(1)}s`, color: "var(--success-text)" };
    case "failed":
      return { text: j.error ?? "失败", color: "var(--danger-text)" };
    default:
      return { text: "排队中", color: "var(--text-faint)" };
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
    <div className="card" style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", overflow: "hidden" }}>
      <div style={{ display: "flex", alignItems: "center", gap: 8, padding: "12px 16px", borderBottom: "1px solid var(--border-subtle)" }}>
        {icon}
        <span style={{ fontWeight: 700, fontSize: 13, color: "var(--text-strong)" }}>{title}</span>
        <span style={{ fontFamily: "var(--font-mono)", fontSize: 11.5, color: "var(--text-faint)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{path}</span>
      </div>
      <div className="vscroll" style={{ flex: 1, overflow: "auto" }}>
        <div
          className="hover-row"
          onDoubleClick={() => onOpenDir("..")}
          style={{ display: "flex", alignItems: "center", gap: 10, padding: "9px 16px", fontSize: 13, color: "var(--text-faint)", cursor: "default" }}
        >
          <Folder size={15} /><span>..</span>
        </div>
        {files.map((f) => (
          <div
            key={f.name}
            className="hover-row"
            onClick={() => onSelect(f.name === selected ? null : f.name)}
            onDoubleClick={() => f.isDir && onOpenDir(f.name)}
            style={{
              display: "flex", alignItems: "center", gap: 10, padding: "9px 16px", cursor: "default", fontSize: 13,
              background: f.name === selected ? "var(--accent-subtle)" : undefined,
            }}
          >
            {fileIcon(f)}
            <span style={{ flex: 1, color: "var(--text-strong)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{f.name}</span>
            <span style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--text-faint)", width: 70, textAlign: "right" }}>
              {f.isDir ? "—" : fmtSize(f.size)}
            </span>
            <span style={{ fontSize: 11, color: "var(--text-faint)", width: 96, textAlign: "right", whiteSpace: "nowrap" }}>{f.modified}</span>
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
    <div style={{ flex: 1, display: "flex", flexDirection: "column", overflow: "hidden", padding: "24px 28px", gap: 16 }}>
      <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
        <div>
          <div className="eyebrow">SFTP / SCP</div>
          <h1 className="screen-title" style={{ fontSize: 22 }}>文件传输</h1>
        </div>
        <span style={{ flex: 1 }} />
        {active ? (
          <Badge color="accent" dot>{active.hostName} 已连接</Badge>
        ) : (
          <Badge color="neutral">未连接 — 先从主机列表建立会话</Badge>
        )}
      </div>

      <div style={{ flex: 1, minHeight: 0, display: "flex", alignItems: "stretch" }}>
        <FilePane
          title="本地"
          icon={<Laptop size={15} />}
          path={t.localPath}
          files={t.localFiles}
          selected={t.localSel}
          onSelect={(n) => t.select("local", n)}
          onOpenDir={(n) => void t.setLocalPath(active?.id ?? null, joinPath(t.localPath, n))}
        />
        <div style={{ width: 56, flex: "none", display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", gap: 10 }}>
          <div
            onClick={() => active && t.localSel && void t.upload(active.id)}
            title="上传所选"
            style={{
              width: 36, height: 36, borderRadius: 99,
              background: active && t.localSel ? "var(--accent)" : "var(--surface-card)",
              border: active && t.localSel ? "none" : "1px solid var(--border-default)",
              color: active && t.localSel ? "var(--text-on-accent)" : "var(--text-faint)",
              display: "flex", alignItems: "center", justifyContent: "center", cursor: "pointer",
              boxShadow: active && t.localSel ? "var(--glow-sm)" : "none",
            }}
          >
            <ArrowRight size={16} />
          </div>
          <div
            onClick={() => active && t.remoteSel && void t.download(active.id)}
            title="下载所选"
            style={{
              width: 36, height: 36, borderRadius: 99,
              background: active && t.remoteSel ? "var(--accent)" : "var(--surface-card)",
              border: active && t.remoteSel ? "none" : "1px solid var(--border-default)",
              color: active && t.remoteSel ? "var(--text-on-accent)" : "var(--text-muted)",
              display: "flex", alignItems: "center", justifyContent: "center", cursor: "pointer",
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

      <div className="card" style={{ flex: "none", padding: "14px 18px", maxHeight: 180, display: "flex", flexDirection: "column" }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 10 }}>
          <span style={{ fontWeight: 700, fontSize: 13, color: "var(--text-strong)" }}>传输队列</span>
          <span style={{ fontSize: 11.5, color: "var(--text-faint)" }}>
            {runningCount} 进行中 · {queuedCount} 排队
          </span>
        </div>
        <div className="vscroll" style={{ display: "flex", flexDirection: "column", gap: 2, overflow: "auto" }}>
          {t.jobs.length === 0 && <div style={{ fontSize: 12, color: "var(--text-faint)", padding: "6px 2px" }}>队列为空 — 选中文件后用中间箭头上传/下载。</div>}
          {t.jobs.map((j) => {
            const meta = jobMeta(j);
            return (
              <div key={j.id} style={{ display: "flex", alignItems: "center", gap: 12, padding: "7px 2px" }}>
                {j.direction === "upload" ? <Upload size={15} /> : <Download size={15} />}
                <span style={{ fontFamily: "var(--font-mono)", fontSize: 12, color: "var(--text-strong)", width: 210, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{j.name}</span>
                <span style={{ fontSize: 11, color: "var(--text-faint)", flex: 1, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>
                  {j.direction === "upload" ? `${j.localPath} → ${j.remotePath}` : `${j.remotePath} → ${j.localPath}`}
                </span>
                <div style={{ width: 180 }}><Progress value={j.progress} /></div>
                <span style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--text-muted)", width: 48, textAlign: "right" }}>
                  {j.state === "queued" ? "—" : `${Math.round(j.progress)}%`}
                </span>
                <span style={{ fontSize: 11, color: meta.color, width: 110, textAlign: "right", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }} title={j.error ?? undefined}>{meta.text}</span>
                {j.state === "failed" && (
                  <span
                    onClick={() => void ipc.retryTransfer(j.id)}
                    style={{ fontSize: 11.5, color: "var(--accent-text)", cursor: "pointer", whiteSpace: "nowrap" }}
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
