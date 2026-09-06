import { ArrowDownToLine, ArrowUpFromLine, X } from "lucide-react";
import { ipc } from "@/ipc";
import { useTransferStore } from "@/stores/transfer";

/** 终端左下角的 ZMODEM 传输浮条 —— 与错误提示条同位,传输期间不会触发错误检测,
 *  二者不会同时出现。 */
export function TransferHint({ sessionId }: { sessionId: string }) {
  const job = useTransferStore((s) =>
    s.jobs.find((j) => j.protocol === "zmodem" && j.sessionId === sessionId && j.state === "running"),
  );
  if (!job) return null;
  const up = job.direction === "upload";
  return (
    <div className="transfer-hint">
      {up ? <ArrowUpFromLine size={13} /> : <ArrowDownToLine size={13} />}
      <span className="grow ellipsis">{job.name}</span>
      <span className="mono t-muted">{Math.round(job.progress)}%</span>
      <span className="mono t-faint">{Math.round(job.rateBps / 1024)} KB/s</span>
      <X size={13} className="clickable" onClick={() => void ipc.zmodemCancel(sessionId)} />
    </div>
  );
}
