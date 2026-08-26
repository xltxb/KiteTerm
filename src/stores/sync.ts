import { create } from "zustand";
import { ipc } from "@/ipc";
import type { SyncConfig, SyncStatus } from "@/types";

interface SyncState {
  status: SyncStatus;
  config: SyncConfig | null;
  wired: boolean;
  load: () => Promise<void>;
  syncNow: () => Promise<void>;
  saveConfig: (cfg: SyncConfig & { secretKey?: string; masterPassword?: string }) => Promise<void>;
  setScope: (key: keyof SyncConfig["scopes"], on: boolean) => Promise<void>;
}

export const useSyncStore = create<SyncState>((set, get) => ({
  status: { configured: false, syncing: false, lastSyncAt: null, devices: [] },
  config: null,
  wired: false,
  load: async () => {
    if (!get().wired) {
      set({ wired: true });
      ipc.onSyncState((status) => set({ status }));
    }
    const [status, config] = await Promise.all([ipc.getSyncStatus(), ipc.getSyncConfig()]);
    set({ status, config });
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
