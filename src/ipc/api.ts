import type {
  AccountInfo,
  AppSettings,
  FileEntry,
  ForwardRule,
  ForwardType,
  Host,
  HostGroup,
  MonitorSample,
  Recording,
  SessionInfo,
  Snippet,
  SshKey,
  SyncConfig,
  SyncStatus,
  TransferJob,
} from "@/types";

export type Unsubscribe = () => void;

/** 前端 ↔ 后端 IPC 契约(《开发文档》2.5)。浏览器环境由 mock 实现。 */
export interface VelaIpc {
  // ---- hosts ----
  listGroups(): Promise<HostGroup[]>;
  listHosts(): Promise<Host[]>;
  saveHost(host: Partial<Host> & { name: string }): Promise<Host>;
  deleteHost(id: string): Promise<void>;

  // ---- keys ----
  listKeys(): Promise<SshKey[]>;
  generateKey(name: string, algo: "ed25519" | "rsa4096"): Promise<SshKey>;
  importKey(name: string, pem: string): Promise<SshKey>;
  getPublicKey(id: string): Promise<string>;
  setKeySync(id: string, enabled: boolean): Promise<void>;
  deleteKey(id: string): Promise<void>;

  // ---- snippets ----
  listSnippets(): Promise<Snippet[]>;
  saveSnippet(s: Partial<Snippet> & { name: string; command: string }): Promise<Snippet>;
  bumpSnippet(id: string): Promise<void>;
  deleteSnippet(id: string): Promise<void>;

  // ---- forwards ----
  listForwards(): Promise<ForwardRule[]>;
  createForward(rule: {
    type: ForwardType;
    localPort: number;
    target: string;
    viaHostId: string;
  }): Promise<ForwardRule>;
  toggleForward(id: string, enabled: boolean): Promise<void>;
  deleteForward(id: string): Promise<void>;
  onForwardStats(cb: (rules: ForwardRule[]) => void): Unsubscribe;

  // ---- sessions / terminal ----
  /** password 仅在主机未配置密钥时需要;错误消息含 PASSWORD_REQUIRED / HOSTKEY_CHANGED 标记 */
  openSession(hostId: string, password?: string): Promise<SessionInfo>;
  /** 主机密钥指纹变更后用户确认信任(清除旧记录,下次连接重新 TOFU) */
  trustHostKey(hostId: string): Promise<void>;
  writeSession(id: string, data: string): Promise<void>;
  resizeSession(id: string, cols: number, rows: number): Promise<void>;
  closeSession(id: string): Promise<void>;
  onSessionData(cb: (id: string, data: string) => void): Unsubscribe;
  onSessionState(cb: (info: SessionInfo) => void): Unsubscribe;

  // ---- sftp / transfer ----
  sftpList(sessionId: string, path: string): Promise<FileEntry[]>;
  localList(path: string): Promise<FileEntry[]>;
  localHome(): Promise<string>;
  enqueueTransfer(req: {
    sessionId: string;
    direction: "upload" | "download";
    localPath: string;
    remotePath: string;
    name: string;
  }): Promise<TransferJob>;
  listTransfers(): Promise<TransferJob[]>;
  retryTransfer(id: string): Promise<void>;
  onTransferProgress(cb: (job: TransferJob) => void): Unsubscribe;

  // ---- recording ----
  startRecording(sessionId: string): Promise<void>;
  stopRecording(sessionId: string): Promise<Recording>;
  listRecordings(): Promise<Recording[]>;
  /** 返回 asciicast v2 原文(首行 header,随后事件行) */
  readRecording(id: string): Promise<string>;

  // ---- host latency ----
  onHostLatency(cb: (latency: Record<string, number | null>) => void): Unsubscribe;

  // ---- monitor ----
  monitorSamples(): Promise<MonitorSample[]>;
  onMonitor(cb: (samples: MonitorSample[]) => void): Unsubscribe;

  // ---- sync(S3 凭据由管理平台下发,本地仅配置分项开关与主密码)----
  getSyncStatus(): Promise<SyncStatus>;
  getSyncConfig(): Promise<SyncConfig>;
  saveSyncConfig(cfg: SyncConfig & { secretKey?: string; masterPassword?: string }): Promise<void>;
  syncNow(): Promise<void>;
  onSyncState(cb: (s: SyncStatus) => void): Unsubscribe;

  // ---- 账户(管理平台)----
  accountRegister(email: string, password: string): Promise<void>;
  accountLogin(email: string, password: string): Promise<AccountInfo>;
  accountLogout(): Promise<void>;
  accountCurrent(): Promise<AccountInfo | null>;
  platformUrlGet(): Promise<string>;
  platformUrlSet(url: string): Promise<void>;

  // ---- settings ----
  getSettings(): Promise<AppSettings>;
  saveSettings(s: AppSettings): Promise<void>;

  // ---- AI 助手 ----
  aiGetConfig(): Promise<{ model: string; hasKey: boolean }>;
  aiSaveConfig(model: string, apiKey?: string): Promise<void>;
  /** messages 为完整对话历史;terminalContext 为当前会话最近输出 */
  aiAsk(messages: { role: "user" | "assistant"; content: string }[], terminalContext: string): Promise<string>;

  // ---- 会话恢复 / 退出 ----
  restoreTargets(): Promise<string[]>;
  appQuit(): Promise<void>;
  onCloseRequested(cb: (activeSessions: number) => void): Unsubscribe;
}

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}
