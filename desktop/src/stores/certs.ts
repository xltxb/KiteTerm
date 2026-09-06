import { create } from "zustand";
import { message as tauriMessage } from "@tauri-apps/plugin-dialog";
import { ipc } from "@/ipc";
import { showError } from "@/lib/dialog";
import type { CertDnsRecord, CertInfo, CertProgress, IssueRequest } from "@/types";

/** 签发共 5 步(校验域名 → 完成挑战 → 申请证书 → 部署 → 重载服务),索引对应 CertProgress.step */
const ISSUE_STEPS = 5;

interface CertsState {
  certs: CertInfo[];
  /** 是否正在签发中(驱动进度面板的显示与轮询态) */
  issuing: boolean;
  /** 五步签发进度,按 step 索引存放各步当前状态 */
  issueSteps: CertProgress[];
  /** DNS-01 挑战需要用户手动添加的解析记录;确认后清空 */
  dnsRecord: CertDnsRecord | null;
  wired: boolean;

  wire: () => void;
  load: () => Promise<void>;
  issue: (req: IssueRequest) => Promise<void>;
  renew: (id: string) => Promise<void>;
  revoke: (id: string) => Promise<void>;
  deploy: (id: string, hostId: string) => Promise<void>;
  confirmDns: (id: string) => Promise<void>;
  downloadChain: (id: string) => Promise<void>;
}

/** 将一条进度事件按 step 索引合并进现有五步数组(替换该步,其余不变) */
export function mergeIssueStep(steps: CertProgress[], p: CertProgress): CertProgress[] {
  const next = steps.slice();
  next[p.step] = p;
  return next;
}

/** 全部五步均已 ok,视为签发流程结束 */
export function isIssueDone(steps: CertProgress[]): boolean {
  if (steps.length < ISSUE_STEPS) return false;
  for (let i = 0; i < ISSUE_STEPS; i++) {
    if (steps[i]?.state !== "ok") return false;
  }
  return true;
}

export const useCertsStore = create<CertsState>((set, get) => ({
  certs: [],
  issuing: false,
  issueSteps: [],
  dnsRecord: null,
  wired: false,

  wire: () => {
    if (get().wired) return;
    set({ wired: true });
    ipc.onCertProgress((p) => {
      set((s) => {
        const issueSteps = mergeIssueStep(s.issueSteps, p);
        const done = p.state === "error" || isIssueDone(issueSteps);
        return { issueSteps, issuing: done ? false : s.issuing };
      });
    });
    ipc.onCertDnsRecord((r) => set({ dnsRecord: r }));
  },

  load: async () => {
    set({ certs: await ipc.certList() });
  },

  issue: async (req) => {
    get().wire();
    set({ issuing: true, issueSteps: [], dnsRecord: null });
    try {
      await ipc.certIssue(req);
      await get().load();
    } catch (e) {
      await showError(`签发证书失败:${String(e)}`);
    } finally {
      // 进度事件也可能已经把 issuing 置为 false,这里再收尾一次,保证不会卡住转圈
      set({ issuing: false });
    }
  },

  renew: async (id) => {
    try {
      await ipc.certRenew(id);
      await get().load();
    } catch (e) {
      await showError(`续期证书失败:${String(e)}`);
    }
  },

  revoke: async (id) => {
    await ipc.certRevoke(id);
    await get().load();
  },

  deploy: async (id, hostId) => {
    try {
      await ipc.certDeploy(id, hostId);
      await get().load();
    } catch (e) {
      await showError(`部署证书失败:${String(e)}`);
    }
  },

  confirmDns: async (id) => {
    await ipc.certDnsConfirm(id);
    set({ dnsRecord: null });
  },

  downloadChain: async (id) => {
    const pem = await ipc.certDownloadChain(id);
    // 应用侧尚未接入文件保存对话框(仅有 open/confirm/message 权限,无 save/fs 写入能力),
    // 这里退化为复制到剪贴板,与仓库内其他「复制」操作(如公钥、片段命令)保持一致体验。
    try {
      await navigator.clipboard.writeText(pem);
      await tauriMessage("证书链已复制到剪贴板", { title: "下载证书链", kind: "info" });
    } catch (e) {
      await showError(`下载证书链失败:${String(e)}`);
    }
  },
}));
