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
  FolderOpen,
  Laptop,
  Server,
  Upload,
} from "lucide-react";
import { Badge, Button, Progress } from "@/components/ui";
import { ipc } from "@/ipc";
import { fmtBytes } from "@/lib/format";
import { pickDirectory } from "@/lib/dialog";
import { useSessionStore } from "@/stores/session";
import { useTransferStore } from "@/stores/transfer";
import { useUiStore } from "@/stores/ui";
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

/** 一侧的文件面板。`path` 为 null 表示这一端还没选定 —— 此时不列目录,
 *  而是把 `chooser` 交出来让用户先挑。 */
function FilePane({
  title,
  icon,
  path,
  files,
  selected,
  chooser,
  onSelect,
  onOpenDir,
  onClose,
}: {
  title: string;
  icon: React.ReactNode;
  path: string | null;
  files: FileEntry[];
  selected: string | null;
  chooser: React.ReactNode;
  onSelect: (name: string | null) => void;
  onOpenDir: (name: string) => void;
  onClose: () => void;
}) {
  if (path === null) {
    return (
      <div className="card col grow clip">
        <div className="row gap-8 pad-tight bd-bottom">
          {icon}
          <span className="w-bold fs-md t-strong">{title}</span>
        </div>
        <div className="grow scroll vscroll">{chooser}</div>
      </div>
    );
  }
  return (
    <div className="card col grow clip">
      <div className="row gap-8 pad-tight bd-bottom">
        {icon}
        <span className="w-bold fs-md t-strong">{title}</span>
        <span className="mono fs-xs t-faint ellipsis grow">{path}</span>
        <span className="fs-xs t-faint clickable nowrap" onClick={onClose} title="重新选择">
          更换
        </span>
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

/** 两端都没选好时,传输箭头是灰的 */
function ArrowButton({
  enabled,
  title,
  onClick,
  children,
}: {
  enabled: boolean;
  title: string;
  onClick: () => void;
  children: React.ReactNode;
}) {
  return (
    <div
      onClick={() => enabled && onClick()}
      title={title}
      className="row jcenter r-full"
      style={{
        width: 36,
        height: 36,
        cursor: enabled ? "pointer" : "not-allowed",
        background: enabled ? "var(--accent)" : "var(--surface-card)",
        border: enabled ? "none" : "1px solid var(--border-default)",
        color: enabled ? "var(--text-on-accent)" : "var(--text-faint)",
        boxShadow: enabled ? "var(--glow-sm)" : "none",
      }}
    >
      {children}
    </div>
  );
}

export function TransferScreen() {
  const sessions = useSessionStore((s) => s.sessions);
  // 传输只认 SSH 会话 —— 本地终端没有远端文件系统,不参与选择
  const sshSessions = sessions.filter((s) => s.kind === "ssh");
  const go = useUiStore((s) => s.go);
  const t = useTransferStore();

  // 只接事件与拉队列 —— 两端都等用户选
  useEffect(() => {
    void t.init();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  // 选中的主机断开后,把远端一侧退回未选状态,免得对着一个已死的会话操作
  useEffect(() => {
    if (t.remoteSessionId && !sessions.some((s) => s.id === t.remoteSessionId)) {
      t.closePane("remote");
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessions, t.remoteSessionId]);

  const remote = sessions.find((s) => s.id === t.remoteSessionId) ?? null;
  const ready = t.localPath !== null && t.remoteSessionId !== null;
  const runningCount = t.jobs.filter((j) => j.state === "running").length;
  const queuedCount = t.jobs.filter((j) => j.state === "queued").length;

  const localChooser = (
    <div className="col gap-12 pad-card">
      <div className="fs-sm t-faint lh-relaxed">选择本地的源/目标目录。</div>
      <div className="row gap-8 wrap">
        <Button
          variant="primary"
          onClick={async () => {
            const dir = await pickDirectory();
            if (dir) await t.openLocal(dir);
          }}
        >
          <FolderOpen size={14} />选择目录…
        </Button>
        <Button variant="secondary" onClick={() => void t.openLocal()}>
          用主目录
        </Button>
      </div>
    </div>
  );

  const remoteChooser = (
    <div className="col gap-12 pad-card">
      {sshSessions.length === 0 ? (
        <>
          <div className="fs-sm t-faint lh-relaxed">
            还没有已连接的主机。先去主机列表建立一个会话,再回来选。
          </div>
          <div>
            <Button variant="secondary" onClick={() => go("hosts")}>浏览主机</Button>
          </div>
        </>
      ) : (
        <>
          <div className="fs-sm t-faint lh-relaxed">选择要传输的主机:</div>
          <div className="col gap-8">
            {sshSessions.map((sn) => (
              <div key={sn.id} className="pick-card" onClick={() => void t.openRemote(sn.id)}>
                <span className={sn.state === "connected" ? "dot-lg dot-ok" : "dot-lg dot-warn"} />
                <div className="grow lh-snug">
                  <div className="fs-md w-semi t-strong">{sn.hostName}</div>
                  <div className="mono fs-xs t-faint">
                    {sn.user}@{sn.addr}:{sn.port}
                  </div>
                </div>
              </div>
            ))}
          </div>
        </>
      )}
    </div>
  );

  return (
    <div className="col grow gap-16 clip py-24 px-28">
      <div className="row gap-12">
        <div>
          <div className="eyebrow">SFTP / SCP</div>
          <h1 className="screen-title fs-xl">文件传输</h1>
        </div>
        <span className="spacer" />
        {ready ? (
          <Badge color="accent" dot>本地 ↔ {remote?.hostName}</Badge>
        ) : (
          <Badge color="neutral">先选定两端</Badge>
        )}
      </div>

      <div className="row grow min-h-0 astretch">
        <FilePane
          title="本地"
          icon={<Laptop size={15} />}
          path={t.localPath}
          files={t.localFiles}
          selected={t.localSel}
          chooser={localChooser}
          onSelect={(n) => t.select("local", n)}
          onOpenDir={(n) => void t.setLocalPath(joinPath(t.localPath ?? "/", n))}
          onClose={() => t.closePane("local")}
        />
        <div className="col fnone jcenter acenter gap-10" style={{ width: 56 }}>
          <ArrowButton
            enabled={ready && !!t.localSel}
            title={ready ? "上传所选" : "先选定两端"}
            onClick={() => void t.upload()}
          >
            <ArrowRight size={16} />
          </ArrowButton>
          <ArrowButton
            enabled={ready && !!t.remoteSel}
            title={ready ? "下载所选" : "先选定两端"}
            onClick={() => void t.download()}
          >
            <ArrowLeft size={16} />
          </ArrowButton>
        </div>
        <FilePane
          title={remote?.hostName ?? "远端"}
          icon={<Server size={15} />}
          path={t.remoteSessionId ? t.remotePath : null}
          files={t.remoteFiles}
          selected={t.remoteSel}
          chooser={remoteChooser}
          onSelect={(n) => t.select("remote", n)}
          onOpenDir={(n) => void t.setRemotePath(joinPath(t.remotePath, n))}
          onClose={() => t.closePane("remote")}
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
                  {j.protocol === "zmodem"
                    ? "ZMODEM"
                    : j.direction === "upload"
                      ? `${j.localPath} → ${j.remotePath}`
                      : `${j.remotePath} → ${j.localPath}`}
                </span>
                <div style={{ width: 180 }}><Progress value={j.progress} /></div>
                <span className="mono fs-xs t-muted text-right" style={{ width: 48 }}>
                  {j.state === "queued" ? "—" : `${Math.round(j.progress)}%`}
                </span>
                <span className={`fs-xs ellipsis text-right ${meta.tone}`} style={{ width: 110 }} title={j.error ?? undefined}>{meta.text}</span>
                {j.state === "failed" && j.protocol !== "zmodem" && (
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
