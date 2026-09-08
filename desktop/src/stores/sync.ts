import { create } from "zustand";
import { ipc } from "@/ipc";
import { useDataStore } from "@/stores/data";
import type { AccountInfo, SyncConfig, SyncStatus, TotpSetup } from "@/types";

interface SyncState {
  status: SyncStatus;
  config: SyncConfig | null;
  account: AccountInfo | null;
  wired: boolean;
  load: () => Promise<void>;
  syncNow: () => Promise<void>;
  /** 具备同步条件时后台跑一次同步;不具备或失败都静默,用户仍可手动重试 */
  autoSync: () => Promise<void>;
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
    // 登录即拉一次:这是用户最期待数据出现的时刻。本机没设主密码时 autoSync 自会跳过
    void get().autoSync();
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
    try {
      await ipc.syncNow();
    } finally {
      // 失败也要把 syncing 归位,否则按钮一直转
      set({ status: await ipc.getSyncStatus() });
      // 拉下来的主机/密钥/片段是后端直接写进 SQLite 的,前端这份快照只在启动时读过一次,
      // 不重载的话界面上什么都不会变 —— 表现就是「同步完切到主机页仍是空的,刷新才出来」。
      // 放在 finally:中途失败时可能已应用了一部分远端条目,那些同样要显示出来。
      await useDataStore.getState().load();
    }
  },
  autoSync: async () => {
    // 未登录、平台停用同步、或本机还没设主密码时,configured 为 false,直接跳过
    if (!(await ipc.getSyncStatus()).configured) return;
    try {
      await get().syncNow();
    } catch {
      /* 离线或桶不可达:静默,保留手动入口 */
    }
  },
  saveConfig: async (cfg) => {
    const had = get().config?.hasMasterKey ?? false;
    await ipc.saveSyncConfig(cfg);
    const [status, config] = await Promise.all([ipc.getSyncStatus(), ipc.getSyncConfig()]);
    set({ status, config });
    // 新设备刚补上主密码 —— 同步条件此刻才凑齐,立刻拉一次,不必再让用户找按钮
    if (!had && config.hasMasterKey) void get().autoSync();
  },
  setScope: async (key, on) => {
    const cfg = get().config;
    if (!cfg) return;
    const next = { ...cfg, scopes: { ...cfg.scopes, [key]: on } };
    set({ config: next });
    await ipc.saveSyncConfig(next);
  },
}));
