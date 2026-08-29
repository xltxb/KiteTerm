import { create } from "zustand";
import { ipc } from "@/ipc";
import type { FileEntry, TransferJob } from "@/types";

/**
 * 文件传输。
 *
 * 两端都由用户显式选定后才列目录 —— 进页面时谁都不连。
 * 早先右栏会偷偷绑上「当前活跃会话」,同时开多台主机时很容易传错机器,
 * 而界面上看不出它绑的是哪一台。
 */
interface TransferState {
  jobs: TransferJob[];

  /** 本地侧:未选目录时为 null */
  localPath: string | null;
  localFiles: FileEntry[];
  localSel: string | null;

  /** 远端侧:未选主机时 remoteSessionId 为 null */
  remoteSessionId: string | null;
  remotePath: string;
  remoteFiles: FileEntry[];
  remoteSel: string | null;

  wired: boolean;

  /** 只接事件与拉队列,不碰任何一端 */
  init: () => Promise<void>;
  /** 选定本地目录(用户从目录选择器或「主目录」入口进来) */
  openLocal: (path?: string) => Promise<void>;
  /** 选定远端主机,并列出其根目录 */
  openRemote: (sessionId: string) => Promise<void>;
  /** 放开某一端,回到未选状态 */
  closePane: (pane: "local" | "remote") => void;

  setLocalPath: (path: string) => Promise<void>;
  setRemotePath: (path: string) => Promise<void>;
  select: (pane: "local" | "remote", name: string | null) => void;
  upload: () => Promise<void>;
  download: () => Promise<void>;
}

export const useTransferStore = create<TransferState>((set, get) => ({
  jobs: [],
  localPath: null,
  localFiles: [],
  localSel: null,
  remoteSessionId: null,
  remotePath: "/",
  remoteFiles: [],
  remoteSel: null,
  wired: false,

  init: async () => {
    if (!get().wired) {
      set({ wired: true });
      ipc.onTransferProgress((job) => {
        set((s) => ({
          jobs: s.jobs.some((j) => j.id === job.id)
            ? s.jobs.map((j) => (j.id === job.id ? job : j))
            : [...s.jobs, job],
        }));
      });
    }
    set({ jobs: await ipc.listTransfers() });
  },

  openLocal: async (path) => {
    const target = path ?? (await ipc.localHome());
    const localFiles = await ipc.localList(target);
    set({ localPath: target, localFiles, localSel: null });
  },

  openRemote: async (sessionId) => {
    const remoteFiles = await ipc.sftpList(sessionId, "/");
    set({ remoteSessionId: sessionId, remotePath: "/", remoteFiles, remoteSel: null });
  },

  closePane: (pane) =>
    pane === "local"
      ? set({ localPath: null, localFiles: [], localSel: null })
      : set({ remoteSessionId: null, remoteFiles: [], remoteSel: null, remotePath: "/" }),

  setLocalPath: async (path) => {
    const localFiles = await ipc.localList(path);
    set({ localPath: path, localFiles, localSel: null });
  },

  setRemotePath: async (path) => {
    const { remoteSessionId } = get();
    if (!remoteSessionId) return;
    const remoteFiles = await ipc.sftpList(remoteSessionId, path);
    set({ remotePath: path, remoteFiles, remoteSel: null });
  },

  select: (pane, name) =>
    pane === "local" ? set({ localSel: name }) : set({ remoteSel: name }),

  upload: async () => {
    const { localSel, localPath, remotePath, remoteSessionId } = get();
    if (!localSel || !localPath || !remoteSessionId) return;
    await ipc.enqueueTransfer({
      sessionId: remoteSessionId,
      direction: "upload",
      name: localSel,
      localPath: localPath + "/" + localSel,
      remotePath: remotePath + "/" + localSel,
    });
    set({ jobs: await ipc.listTransfers() });
  },

  download: async () => {
    const { remoteSel, localPath, remotePath, remoteSessionId } = get();
    if (!remoteSel || !localPath || !remoteSessionId) return;
    await ipc.enqueueTransfer({
      sessionId: remoteSessionId,
      direction: "download",
      name: remoteSel,
      localPath: localPath + "/" + remoteSel,
      remotePath: remotePath + "/" + remoteSel,
    });
    set({ jobs: await ipc.listTransfers() });
  },
}));
