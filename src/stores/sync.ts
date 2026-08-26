import { create } from "zustand";
import { ipc } from "@/ipc";
import type { AccountInfo, SyncConfig, SyncStatus } from "@/types";

interface SyncState {
  status: SyncStatus;
  config: SyncConfig | null;
  account: AccountInfo | null;
  wired: boolean;
  load: () => Promise<void>;
  syncNow: () => Promise<void>;
  saveConfig: (cfg: SyncConfig & { secretKey?: string; masterPassword?: string }) => Promise<void>;
  setScope: (key: keyof SyncConfig["scopes"], on: boolean) => Promise<void>;
  login: (email: string, password: string) => Promise<void>;
  register: (email: string, password: string) => Promise<void>;
  logout: () => Promise<void>;
}

export const useSyncStore = create<SyncState>((set, get) => ({
  status: { configured: false, syncing: false, lastSyncAt: null, devices: [] },
  config: null,
  account: null,
  wired: false,
  load: async () => {
    if (!get().wired) {
      set({ wired: true });
      ipc.onSyncState((status) => set({ status }));
    }
    const [status, config, account] = await Promise.all([
      ipc.getSyncStatus(),
      ipc.getSyncConfig(),
      ipc.accountCurrent(),
    ]);
    set({ status, config, account });
  },
  login: async (email, password) => {
    const account = await ipc.accountLogin(email, password);
    set({ account, status: await ipc.getSyncStatus() });
  },
  register: async (email, password) => {
    await ipc.accountRegister(email, password);
    await get().login(email, password);
  },
  logout: async () => {
    await ipc.accountLogout();
    set({ account: null, status: await ipc.getSyncStatus() });
  },
  syncNow: async () => {
    set((s) => ({ status: { ...s.status, syncing: true } }));
    await ipc.syncNow();
    set({ status: await ipc.getSyncStatus() });
  },
  saveConfig: async (cfg) => {
    await ipc.saveSyncConfig(cfg);
    const [status, config] = await Promise.all([ipc.getSyncStatus(), ipc.getSyncConfig()]);
    set({ status, config });
  },
  setScope: async (key, on) => {
    const cfg = get().config;
    if (!cfg) return;
    const next = { ...cfg, scopes: { ...cfg.scopes, [key]: on } };
    set({ config: next });
    await ipc.saveSyncConfig(next);
  },
}));
