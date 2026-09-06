import { create } from "zustand";
import { ipc } from "@/ipc";
import type { AccountInfo, SyncConfig, SyncStatus, TotpSetup } from "@/types";

interface SyncState {
  status: SyncStatus;
  config: SyncConfig | null;
  account: AccountInfo | null;
  wired: boolean;
  load: () => Promise<void>;
  syncNow: () => Promise<void>;
  saveConfig: (cfg: SyncConfig & { secretKey?: string; masterPassword?: string }) => Promise<void>;
  setScope: (key: keyof SyncConfig["scopes"], on: boolean) => Promise<void>;
  login: (email: string, password: string, totp?: string) => Promise<void>;
  register: (email: string, password: string, inviteCode?: string) => Promise<void>;
  logout: () => Promise<void>;
  /** 向平台核对一次账户状态;离线等非 LOGIN_REQUIRED 错误静默吞掉 */
  refreshAccount: () => Promise<void>;
  verifyEmail: (code: string) => Promise<void>;
  resendVerification: () => Promise<void>;
  totpSetup: () => Promise<TotpSetup>;
  totpEnable: (code: string) => Promise<void>;
  totpDisable: (code: string) => Promise<void>;
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
    // 启动时核对一次,拿到多设备被挤下线/管理员改动的最新状态
    if (account) void get().refreshAccount();
  },
  login: async (email, password, totp) => {
    const account = await ipc.accountLogin(email, password, totp);
    set({ account, status: await ipc.getSyncStatus() });
  },
  register: async (email, password, inviteCode) => {
    // 平台注册成功即返回令牌,无需再登录一次
    const account = await ipc.accountRegister(email, password, inviteCode);
    set({ account, status: await ipc.getSyncStatus() });
  },
  logout: async () => {
    await ipc.accountLogout();
    set({ account: null, status: await ipc.getSyncStatus() });
  },
  refreshAccount: async () => {
    try {
      set({ account: await ipc.accountRefresh() });
    } catch (e) {
      if (String(e).includes("LOGIN_REQUIRED")) set({ account: null });
    }
  },
  verifyEmail: async (code) => set({ account: await ipc.accountVerifyEmail(code) }),
  resendVerification: () => ipc.accountResendVerification(),
  totpSetup: () => ipc.accountTotpSetup(),
  totpEnable: async (code) => set({ account: await ipc.accountTotpEnable(code) }),
  totpDisable: async (code) => set({ account: await ipc.accountTotpDisable(code) }),
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
