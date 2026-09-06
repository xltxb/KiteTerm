// 领域类型 — 与 src-tauri SQLite 模型及 IPC 契约一一对应(见《KiteTerm 产品与开发文档》3.3)

export type Screen =
  | "terminal"
  | "hosts"
  | "transfer"
  | "forwards"
  | "monitor"
  | "keys"
  | "snippets"
  | "sync"
  | "settings";

export interface HostGroup {
  id: string;
  name: string;
  sort: number;
}

export interface Host {
  id: string;
  name: string;
  addr: string;
  port: number;
  user: string;
  groupId: string;
  tags: string[];
  keyId: string | null;
  lastConnectedAt: string | null;
  /** 实时延迟(ms),由后端探测事件更新;null = 未知 */
  latencyMs: number | null;
}

export interface SshKey {
  id: string;
  name: string;
  algo: "ed25519" | "rsa4096";
  fingerprint: string;
  publicKey: string;
  syncEnabled: boolean;
  createdAt: string;
  /** 使用该密钥的主机数(派生字段) */
  usedByHosts: number;
}

export interface Snippet {
  id: string;
  name: string;
  command: string;
  tags: string[];
  useCount: number;
  updatedAt: string;
}

export type ForwardType = "L" | "R" | "D";

export interface ForwardRule {
  id: string;
  type: ForwardType;
  localPort: number;
  /** 目标 host:port;D 类为空 */
  target: string;
  viaHostId: string;
  enabled: boolean;
  bytesUp: number;
  bytesDown: number;
}

export interface Recording {
  id: string;
  sessionHost: string;
  name: string;
  durationMs: number;
  startedAt: string;
  syncEnabled: boolean;
}

export type SessionState = "connecting" | "connected" | "disconnected";

export interface SessionInfo {
  id: string;
  hostId: string;
  hostName: string;
  user: string;
  addr: string;
  port: number;
  state: SessionState;
  latencyMs: number | null;
  cipher: string;
  cols: number;
  rows: number;
  recording: boolean;
  recordingSince: number | null;
}

export interface FileEntry {
  name: string;
  isDir: boolean;
  size: number;
  modified: string;
}

export type TransferState = "queued" | "running" | "done" | "failed";

export interface TransferJob {
  id: string;
  direction: "upload" | "download";
  name: string;
  localPath: string;
  remotePath: string;
  sessionId: string;
  progress: number; // 0-100
  rateBps: number;
  state: TransferState;
  error: string | null;
  elapsedMs: number;
}

export interface MonitorSample {
  hostId: string;
  hostName: string;
  online: boolean;
  uptime: string;
  cpu: number;
  mem: number;
  disk: number;
  load: string;
  netUp: string;
  netDown: string;
}

export interface SyncConfig {
  endpoint: string;
  bucket: string;
  region: string;
  accessKey: string;
  /** 是否已配置 secret(secret 本身不回传前端) */
  hasSecret: boolean;
  /** 是否已设置主密码(派生密钥已缓存) */
  hasMasterKey: boolean;
  scopes: { hosts: boolean; keys: boolean; snippets: boolean; settings: boolean };
}

/** AI 模型配置(密钥不回传前端) */
export interface AiProvider {
  id: string;
  name: string;
  /** anthropic | openai | ollama —— 决定请求哪种接口 */
  protocol: "anthropic" | "openai" | "ollama";
  baseUrl: string;
  model: string;
  hasKey: boolean;
}

export interface AiSettings {
  providers: AiProvider[];
  activeId: string | null;
}

/** AI 协助场景 */
export type AiMode = "chat" | "explain" | "command";

/** 管理平台账户 */
/** 管理平台下发的功能开关。
 *
 *  `enforced` 是门禁总闸(平台的 KITE_FEATURE_GATING),false 时下面三项恒为 true。
 *  `known=false` 表示从未成功拉到过 —— 此时按未启用门禁处理,不限制任何功能。 */
export interface Features {
  /** 注册策略:open | invite | closed。与门禁总闸无关,始终由平台准入政策决定 */
  registration: string;
  /** 平台是否启用了功能门禁 */
  enforced: boolean;
  login: boolean;
  sync: boolean;
  ai: boolean;
  /** 平台下发,安全策略仅提示 */
  keySync: boolean;
  recordingSync: boolean;
  aiCommand: boolean;
  aiDiagnose: boolean;
  aiSummary: boolean;
  emailVerify: boolean;
  totpRequired: boolean;
  multiDevice: boolean;
  /** 口令规则随开关一起下发 */
  passwordMinLength: number;
  passwordRule: string;
  known: boolean;
}

export interface AccountInfo {
  id: string;
  email: string;
  /** 平台是否放开该账户的云同步 */
  syncEnabled: boolean;
}

export interface SyncStatus {
  configured: boolean;
  syncing: boolean;
  lastSyncAt: string | null;
  devices: SyncDevice[];
}

export interface SyncDevice {
  id: string;
  name: string;
  kind: "laptop" | "desktop" | "phone";
  lastSeenAt: string;
  current: boolean;
}

export interface AppSettings {
  theme: "dark" | "light";
  terminalFont: string;
  terminalFontSize: number;
  cursorBlink: boolean;
  restoreSessions: boolean;
  autoReconnect: boolean;
  confirmOnQuit: boolean;
  /** 终端里直接输入白话时,自动转交 AI 生成命令 */
  naturalLanguageInput: boolean;
}

export const DEFAULT_SETTINGS: AppSettings = {
  theme: "dark",
  terminalFont: "JetBrains Mono",
  terminalFontSize: 13,
  cursorBlink: true,
  restoreSessions: true,
  autoReconnect: true,
  confirmOnQuit: false,
  naturalLanguageInput: true,
};
