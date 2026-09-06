import type {
  AccountInfo,
  AiMode,
  AiProvider,
  AiSettings,
  AppSettings,
  FileEntry,
  ForwardRule,
  ForwardType,
  Features,
  Host,
  HostGroup,
  MonitorSample,
  Recording,
  SessionInfo,
  Snippet,
  SshKey,
  SyncConfig,
  SyncStatus,
  TotpSetup,
  TransferJob,
} from "@/types";

export type Unsubscribe = () => void;

/** 前端 ↔ Rust 核心的 IPC 契约(《开发文档》2.5)。唯一实现为 Tauri(桌面端)。 */
export interface KiteIpc {
  // ---- hosts ----
  listGroups(): Promise<HostGroup[]>;
  /** 新建分组(同名幂等) */
  createGroup(name: string): Promise<HostGroup>;
  renameGroup(id: string, name: string): Promise<void>;
  /** 删除分组:组内主机迁到其他分组;至少保留一个分组 */
  deleteGroup(id: string): Promise<void>;
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
  setRecordingSync(id: string, enabled: boolean): Promise<void>;

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
  /** 主密钥的 BIP39 24 词恢复短语(需已设置主密码) */
  syncRecoveryPhrase(): Promise<string>;
  /** 用恢复短语重建主密钥(忘记主密码时) */
  syncRestoreMaster(phrase: string): Promise<void>;

  // ---- 账户(管理平台)----
  /** 平台注册策略:open 开放 / invite 需邀请码 / closed 关闭 */
  // ---- 功能开关(管理平台下发,客户端缓存)----
  /** 读缓存,不发网络请求 */
  featuresGet(): Promise<Features>;
  /** 向平台拉一次并刷新缓存;平台不可达时沿用缓存 */
  featuresRefresh(): Promise<Features>;

  accountRegisterPolicy(): Promise<{ mode: string; passwordMinLength: number; passwordRule: string }>;
  /** 注册成功即完成登录(平台直接签发令牌) */
  accountRegister(email: string, password: string, inviteCode?: string): Promise<AccountInfo>;
  accountLogin(email: string, password: string, totp?: string): Promise<AccountInfo>;
  accountLogout(): Promise<void>;
  accountCurrent(): Promise<AccountInfo | null>;
  /** 提交邮箱验证码;成功后返回刷新后的账户信息 */
  accountVerifyEmail(code: string): Promise<AccountInfo>;
  /** 重新发送邮箱验证码 */
  accountResendVerification(): Promise<void>;
  /** 生成 TOTP 密钥与 otpauth URL(供二维码展示) */
  accountTotpSetup(): Promise<TotpSetup>;
  /** 校验验证码后正式启用 TOTP */
  accountTotpEnable(code: string): Promise<AccountInfo>;
  /** 校验验证码后关闭 TOTP */
  accountTotpDisable(code: string): Promise<AccountInfo>;
  /** 向平台核对一次账户状态(GET /api/me);令牌失效抛 LOGIN_REQUIRED */
  accountRefresh(): Promise<AccountInfo>;
  platformUrlGet(): Promise<string>;
  /** 保存并返回规范化后的地址(补协议、去尾斜杠);格式非法或公网 http 会抛错 */
  platformUrlSet(url: string): Promise<string>;

  // ---- settings ----
  getSettings(): Promise<AppSettings>;
  saveSettings(s: AppSettings): Promise<void>;

  // ---- AI 助手(多模型可选,按协议请求不同接口)----
  aiGetSettings(): Promise<AiSettings>;
  /** 新增或更新模型;apiKey 省略表示保持原密钥,传空串表示清除 */
  aiSaveProvider(provider: Omit<AiProvider, "hasKey"> & { hasKey?: boolean }, apiKey?: string): Promise<AiSettings>;
  aiDeleteProvider(id: string): Promise<AiSettings>;
  aiSetActive(id: string): Promise<AiSettings>;
  /** messages 为完整对话历史;terminalContext 为当前会话最近输出;mode 决定协助场景 */
  aiAsk(
    messages: { role: "user" | "assistant"; content: string }[],
    terminalContext: string,
    mode?: AiMode,
  ): Promise<string>;

  // ---- 会话恢复 / 退出 ----
  restoreTargets(): Promise<string[]>;
  appQuit(): Promise<void>;
  onCloseRequested(cb: (activeSessions: number) => void): Unsubscribe;
}

/** 是否运行在桌面外壳内。仅用于启动守卫 —— 应用不提供浏览器运行形态。 */
export function inDesktopShell(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}
