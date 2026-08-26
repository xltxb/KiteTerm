import { create } from "zustand";
import { ipc } from "@/ipc";
import type { FileEntry, TransferJob } from "@/types";

interface TransferState {
  jobs: TransferJob[];
  localPath: string;
  remotePath: string;
  localFiles: FileEntry[];
  remoteFiles: FileEntry[];
  localSel: string | null;
  remoteSel: string | null;
  wired: boolean;
  init: (sessionId: string | null) => Promise<void>;
  setLocalPath: (sessionId: string | null, path: string) => Promise<void>;
  setRemotePath: (sessionId: string | null, path: string) => Promise<void>;
  select: (pane: "local" | "remote", name: string | null) => void;
  upload: (sessionId: string) => Promise<void>;
  download: (sessionId: string) => Promise<void>;
}

export const useTransferStore = create<TransferState>((set, get) => ({
  jobs: [],
  localPath: "",
  remotePath: "/",
  localFiles: [],
  remoteFiles: [],
  localSel: null,
  remoteSel: null,
  wired: false,

  init: async (sessionId) => {
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
    const home = get().localPath || (await ipc.localHome());
    const [jobs, localFiles] = await Promise.all([ipc.listTransfers(), ipc.localList(home)]);
    set({ jobs, localFiles, localPath: home });
    if (sessionId) {
      const remoteFiles = await ipc.sftpList(sessionId, get().remotePath);
      set({ remoteFiles });
    }
  },

  setLocalPath: async (_sessionId, path) => {
    const localFiles = await ipc.localList(path);
    set({ localPath: path, localFiles, localSel: null });
  },

  setRemotePath: async (sessionId, path) => {
    if (!sessionId) return;
    const remoteFiles = await ipc.sftpList(sessionId, path);
    set({ remotePath: path, remoteFiles, remoteSel: null });
  },

  select: (pane, name) =>
    pane === "local" ? set({ localSel: name }) : set({ remoteSel: name }),

  upload: async (sessionId) => {
    const { localSel, localPath, remotePath } = get();
    if (!localSel) return;
    await ipc.enqueueTransfer({
      sessionId,
      direction: "upload",
      name: localSel,
      localPath: localPath + "/" + localSel,
      remotePath: remotePath + "/" + localSel,
    });
    set({ jobs: await ipc.listTransfers() });
  },

  download: async (sessionId) => {
    const { remoteSel, localPath, remotePath } = get();
    if (!remoteSel) return;
    await ipc.enqueueTransfer({
      sessionId,
      direction: "download",
      name: remoteSel,
      localPath: localPath + "/" + remoteSel,
      remotePath: remotePath + "/" + remoteSel,
    });
    set({ jobs: await ipc.listTransfers() });
  },
}));
