// 浏览器开发模式(无 Tauri)下的内存版后端 — 数据不落盘,仅供 UI 预览联调
import type {
  AppSettings,
  FileEntry,
  ForwardRule,
  Recording,
  SessionInfo,
  Snippet,
  SshKey,
  SyncConfig,
  SyncStatus,
  TransferJob,
} from "@/types";
import { DEFAULT_SETTINGS } from "@/types";
import type { Unsubscribe, VelaIpc } from "./api";
import {
  mockDevices,
  mockForwards,
  mockGroups,
  mockHosts,
  mockKeys,
  mockLocalFiles,
  mockMonitor,
  mockRecordings,
  mockRemoteFiles,
  mockSnippets,
} from "@/mock/data";

type Listener<T extends unknown[]> = (...args: T) => void;

class Emitter<T extends unknown[]> {
  private listeners = new Set<Listener<T>>();
  on(cb: Listener<T>): Unsubscribe {
    this.listeners.add(cb);
    return () => this.listeners.delete(cb);
  }
  emit(...args: T) {
    this.listeners.forEach((l) => l(...args));
  }
}

const uid = () => Math.random().toString(36).slice(2, 10);

/** 极简假 shell:带行内光标(左右移动、中间插入/删除),回车后输出演示内容 */
class FakeShell {
  private line = "";
  private pos = 0;
  private esc = ""; // 正在累积的转义序列
  constructor(
    private id: string,
    private host: { name: string; user: string },
    private emit: (id: string, data: string) => void,
  ) {}
  banner() {
    const d = [
      `Last login: Tue Aug 26 09:12:44 2026 from 10.0.0.8\r\n`,
      `\x1b[90m[mock] 浏览器预览模式 — 输入命令按回车试试\x1b[0m\r\n`,
    ].join("");
    this.emit(this.id, d + this.prompt());
  }
  private prompt() {
    return `\x1b[96m${this.host.user}@${this.host.name}\x1b[0m\x1b[90m:\x1b[0m\x1b[94m~\x1b[0m\x1b[90m$\x1b[0m `;
  }
  /** 重绘当前行并把光标放回 pos 处 */
  private redraw() {
    const back = "\x1b[D".repeat(this.line.length - this.pos);
    this.emit(this.id, "\r\x1b[K" + this.prompt() + this.line + back);
  }
  write(data: string) {
    for (const ch of data) {
      if (this.esc) {
        this.esc += ch;
        // CSI 序列以 @-~ 范围的终结字节结束
        if (this.esc.length >= 3 && ch >= "@" && ch <= "~") {
          if (this.esc === "\x1b[D" && this.pos > 0) {
            this.pos -= 1;
            this.emit(this.id, "\x1b[D");
          } else if (this.esc === "\x1b[C" && this.pos < this.line.length) {
            this.pos += 1;
            this.emit(this.id, "\x1b[C");
          }
          this.esc = "";
        }
        continue;
      }
      if (ch === "\x1b") {
        this.esc = ch;
      } else if (ch === "\r") {
        this.emit(this.id, "\r\n" + this.run(this.line.trim()) + this.prompt());
        this.line = "";
        this.pos = 0;
      } else if (ch === "\x7f") {
        if (this.pos > 0) {
          this.line = this.line.slice(0, this.pos - 1) + this.line.slice(this.pos);
          this.pos -= 1;
          this.redraw();
        }
      } else if (ch >= " ") {
        this.line = this.line.slice(0, this.pos) + ch + this.line.slice(this.pos);
        this.pos += 1;
        if (this.pos === this.line.length) this.emit(this.id, ch);
        else this.redraw();
      }
    }
  }
  private run(cmd: string): string {
    if (!cmd) return "";
    if (cmd === "ls") return "dist  assets  deploy.sh  release-v2.4.tar.gz\r\n";
    if (cmd.startsWith("echo ")) return cmd.slice(5) + "\r\n";
    if (cmd === "uptime") return "09:14  up 42 days, load average: 0.84 0.71 0.66\r\n";
    return `\x1b[90m[mock] 已执行:\x1b[0m ${cmd}\r\n`;
  }
}

export function createMockIpc(): VelaIpc {
  const groups = [...mockGroups];
  let hosts = [...mockHosts];
  let keys: SshKey[] = [...mockKeys];
  let snippets: Snippet[] = [...mockSnippets];
  let forwards: ForwardRule[] = [...mockForwards];
  const recordings: Recording[] = [...mockRecordings];
  let transfers: TransferJob[] = [
    { id: "t1", direction: "upload", name: "release-v2.4.tar.gz", localPath: "~/projects/vela", remotePath: "/var/www/vela/releases", sessionId: "", progress: 68, rateBps: 13_002_342, state: "running", error: null, elapsedMs: 4_200 },
    { id: "t2", direction: "download", name: "realtime.log", localPath: "~/Downloads", remotePath: "/var/www/vela/logs", sessionId: "", progress: 100, rateBps: 0, state: "done", error: null, elapsedMs: 9_200 },
    { id: "t3", direction: "upload", name: ".env.production", localPath: "~/projects/vela", remotePath: "/var/www/vela/shared", sessionId: "", progress: 0, rateBps: 0, state: "queued", error: null, elapsedMs: 0 },
  ];
  let settings: AppSettings = { ...DEFAULT_SETTINGS };
  let aiConfig = { model: "claude-opus-5", hasKey: false };
  let syncStatus: SyncStatus = { configured: true, syncing: false, lastSyncAt: "2 分钟前", devices: mockDevices };
  let syncConfig: SyncConfig = {
    endpoint: "http://127.0.0.1:8787(平台下发)",
    bucket: "vela-sync",
    region: "auto",
    accessKey: "",
    hasSecret: true,
    hasMasterKey: true,
    scopes: { hosts: true, keys: true, snippets: true, settings: false },
  };
  let account: { id: string; email: string; syncEnabled: boolean } | null = {
    id: "u-demo",
    email: "lin@vela.dev",
    syncEnabled: true,
  };
  let platformUrl = "http://127.0.0.1:8787";

  const sessions = new Map<string, SessionInfo>();
  const shells = new Map<string, FakeShell>();
  const evData = new Emitter<[string, string]>();
  const evState = new Emitter<[SessionInfo]>();
  const evTransfer = new Emitter<[TransferJob]>();
  const evMonitor = new Emitter<[typeof mockMonitor]>();
  const evSync = new Emitter<[SyncStatus]>();
  const evForward = new Emitter<[ForwardRule[]]>();
  const evLatency = new Emitter<[Record<string, number | null>]>();

  // 模拟主机延迟探测
  setInterval(() => {
    const m: Record<string, number | null> = {};
    hosts.forEach((h) => {
      m[h.id] = h.latencyMs == null ? null : Math.max(3, h.latencyMs + Math.round(Math.random() * 8 - 4));
    });
    evLatency.emit(m);
  }, 8000);

  // 模拟传输进度
  setInterval(() => {
    const running = transfers.find((t) => t.state === "running");
    if (running && running.progress < 100) {
      running.progress = Math.min(100, running.progress + 4);
      running.elapsedMs += 1000;
      if (running.progress >= 100) {
        running.state = "done";
        running.rateBps = 0;
        const queued = transfers.find((t) => t.state === "queued");
        if (queued) queued.state = "running";
      }
      evTransfer.emit({ ...running });
    }
  }, 1000);

  // 模拟监控刷新
  setInterval(() => {
    mockMonitor.forEach((m) => {
      m.cpu = Math.max(2, Math.min(97, m.cpu + Math.round(Math.random() * 8 - 4)));
    });
    evMonitor.emit([...mockMonitor]);
  }, 5000);

  return {
    async listGroups() { return groups; },
    async listHosts() { return hosts; },
    async saveHost(h) {
      if (h.id) {
        hosts = hosts.map((x) => (x.id === h.id ? { ...x, ...h } as never : x));
        return hosts.find((x) => x.id === h.id)!;
      }
      const nh = {
        id: uid(), name: h.name, addr: h.addr ?? "", port: h.port ?? 22, user: h.user ?? "root",
        groupId: h.groupId ?? groups[0].id, tags: h.tags ?? [], keyId: h.keyId ?? null,
        lastConnectedAt: null, latencyMs: null,
      };
      hosts = [...hosts, nh];
      return nh;
    },
    async deleteHost(id) { hosts = hosts.filter((h) => h.id !== id); },

    async listKeys() { return keys; },
    async generateKey(name, algo) {
      const k: SshKey = {
        id: uid(), name, algo,
        fingerprint: "SHA256:" + btoa(uid() + uid()).slice(0, 43),
        publicKey: (algo === "ed25519" ? "ssh-ed25519 AAAA" : "ssh-rsa AAAA") + uid(),
        syncEnabled: false, createdAt: "2026-08-26", usedByHosts: 0,
      };
      keys = [k, ...keys];
      return k;
    },
    async importKey(name) {
      const k: SshKey = {
        id: uid(), name, algo: "ed25519",
        fingerprint: "SHA256:" + btoa(uid() + uid()).slice(0, 43),
        publicKey: "ssh-ed25519 AAAA" + uid(), syncEnabled: false,
        createdAt: "2026-08-26", usedByHosts: 0,
      };
      keys = [k, ...keys];
      return k;
    },
    async getPublicKey(id) { return keys.find((k) => k.id === id)?.publicKey ?? ""; },
    async setKeySync(id, enabled) {
      keys = keys.map((k) => (k.id === id ? { ...k, syncEnabled: enabled } : k));
    },
    async deleteKey(id) { keys = keys.filter((k) => k.id !== id); },

    async listSnippets() { return snippets; },
    async saveSnippet(s) {
      if (s.id) {
        snippets = snippets.map((x) => (x.id === s.id ? { ...x, ...s } as Snippet : x));
        return snippets.find((x) => x.id === s.id)!;
      }
      const ns: Snippet = { id: uid(), name: s.name, command: s.command, tags: s.tags ?? [], useCount: 0, updatedAt: "2026-08-26" };
      snippets = [ns, ...snippets];
      return ns;
    },
    async bumpSnippet(id) {
      snippets = snippets.map((s) => (s.id === id ? { ...s, useCount: s.useCount + 1 } : s));
    },
    async deleteSnippet(id) { snippets = snippets.filter((s) => s.id !== id); },

    async listForwards() { return forwards; },
    async createForward(rule) {
      const f: ForwardRule = { id: uid(), ...rule, enabled: true, bytesUp: 0, bytesDown: 0 };
      forwards = [...forwards, f];
      return f;
    },
    async toggleForward(id, enabled) {
      forwards = forwards.map((f) => (f.id === id ? { ...f, enabled } : f));
      evForward.emit(forwards);
    },
    async deleteForward(id) { forwards = forwards.filter((f) => f.id !== id); },
    onForwardStats: (cb) => evForward.on(cb),

    async openSession(hostId, _password) {
      const host = hosts.find((h) => h.id === hostId);
      if (!host) throw new Error("host not found: " + hostId);
      const info: SessionInfo = {
        id: uid(), hostId, hostName: host.name, user: host.user, addr: host.addr, port: host.port,
        state: "connected", latencyMs: host.latencyMs ?? 12, cipher: "chacha20-poly1305",
        cols: 120, rows: 32, recording: false, recordingSince: null,
      };
      sessions.set(info.id, info);
      const shell = new FakeShell(info.id, host, (id, data) => evData.emit(id, data));
      shells.set(info.id, shell);
      setTimeout(() => shell.banner(), 50);
      evState.emit(info);
      return info;
    },
    async writeSession(id, data) { shells.get(id)?.write(data); },
    async resizeSession(id, cols, rows) {
      const s = sessions.get(id);
      if (s) { s.cols = cols; s.rows = rows; }
    },
    async closeSession(id) {
      const s = sessions.get(id);
      if (s) { s.state = "disconnected"; evState.emit({ ...s }); }
      sessions.delete(id);
      shells.delete(id);
    },
    onSessionData: (cb) => evData.on(cb),
    onSessionState: (cb) => evState.on(cb),

    async sftpList() { return mockRemoteFiles as FileEntry[]; },
    async localList() { return mockLocalFiles as FileEntry[]; },
    async localHome() { return "~/projects/vela"; },
    async enqueueTransfer(req) {
      const job: TransferJob = {
        id: uid(), direction: req.direction, name: req.name, localPath: req.localPath,
        remotePath: req.remotePath, sessionId: req.sessionId,
        progress: 0, rateBps: 0,
        state: transfers.some((t) => t.state === "running") ? "queued" : "running",
        error: null, elapsedMs: 0,
      };
      transfers = [...transfers, job];
      evTransfer.emit(job);
      return job;
    },
    async listTransfers() { return transfers; },
    async retryTransfer(id) {
      transfers = transfers.map((t) => (t.id === id ? { ...t, state: "queued", error: null } : t));
    },
    onTransferProgress: (cb) => evTransfer.on(cb),

    async startRecording(sessionId) {
      const s = sessions.get(sessionId);
      if (s) { s.recording = true; s.recordingSince = Date.now(); evState.emit({ ...s }); }
    },
    async stopRecording(sessionId) {
      const s = sessions.get(sessionId);
      const rec: Recording = {
        id: uid(), sessionHost: s?.hostName ?? "unknown", name: "新录制",
        durationMs: s?.recordingSince ? Date.now() - s.recordingSince : 0,
        startedAt: "刚刚", syncEnabled: false,
      };
      if (s) { s.recording = false; s.recordingSince = null; evState.emit({ ...s }); }
      recordings.unshift(rec);
      return rec;
    },
    async listRecordings() { return recordings; },
    async readRecording(id) {
      const rec = recordings.find((r) => r.id === id);
      const name = rec?.sessionHost ?? "demo";
      // 演示用的迷你 asciicast v2
      const lines = [
        JSON.stringify({ version: 2, width: 100, height: 28, timestamp: 1756171200 }),
        JSON.stringify([0.3, "o", `\u001b[96mlin@${name}\u001b[0m\u001b[90m:\u001b[0m\u001b[94m~\u001b[0m\u001b[90m$\u001b[0m `]),
        JSON.stringify([1.1, "o", "p"]), JSON.stringify([1.2, "o", "m"]), JSON.stringify([1.3, "o", "2"]),
        JSON.stringify([1.5, "o", " restart gateway\r\n"]),
        JSON.stringify([2.4, "o", "\u001b[92m[PM2] ✓ gateway 已重启 · pid 21840\u001b[0m\r\n"]),
        JSON.stringify([3.0, "o", `\u001b[96mlin@${name}\u001b[0m\u001b[90m:\u001b[0m\u001b[94m~\u001b[0m\u001b[90m$\u001b[0m tail -f logs/realtime.log\r\n`]),
        JSON.stringify([3.8, "o", "\u001b[90m09:14:02\u001b[0m \u001b[92mINFO\u001b[0m  ws    connected clients=1,204\r\n"]),
        JSON.stringify([4.9, "o", "\u001b[90m09:14:05\u001b[0m \u001b[92mINFO\u001b[0m  sync  p50=12ms p99=48ms\r\n"]),
        JSON.stringify([6.2, "o", "\u001b[90m09:14:11\u001b[0m \u001b[93mWARN\u001b[0m  quota project=aurora usage=60%\r\n"]),
      ];
      return lines.join("\n");
    },

    async trustHostKey() {},
    onHostLatency: (cb) => evLatency.on(cb),

    async monitorSamples() { return mockMonitor; },
    onMonitor: (cb) => evMonitor.on(cb),

    async getSyncStatus() { return syncStatus; },
    async getSyncConfig() { return syncConfig; },
    async saveSyncConfig(cfg) {
      syncConfig = { ...cfg, hasSecret: cfg.hasSecret || !!cfg.secretKey };
      syncStatus = { ...syncStatus, configured: true };
    },
    async syncNow() {
      syncStatus = { ...syncStatus, syncing: true };
      evSync.emit(syncStatus);
      setTimeout(() => {
        syncStatus = { ...syncStatus, syncing: false, lastSyncAt: "刚刚" };
        evSync.emit(syncStatus);
      }, 1600);
    },
    onSyncState: (cb) => evSync.on(cb),
    async syncRecoveryPhrase() {
      return "abandon ability able about above absent absorb abstract absurd abuse access accident account accuse achieve acid acoustic acquire across act action actor actress actual";
    },
    async syncRestoreMaster() {},

    async accountRegister() {},
    async accountLogin(email) {
      account = { id: "u-demo", email, syncEnabled: true };
      syncStatus = { ...syncStatus, configured: true };
      return account;
    },
    async accountLogout() {
      account = null;
      syncStatus = { ...syncStatus, configured: false };
    },
    async accountCurrent() { return account; },
    async platformUrlGet() { return platformUrl; },
    async platformUrlSet(url) { platformUrl = url; },

    async getSettings() { return settings; },
    async saveSettings(s) { settings = s; },

    async aiGetConfig() { return aiConfig; },
    async aiSaveConfig(model, apiKey) {
      aiConfig = { model: model || "claude-opus-5", hasKey: aiConfig.hasKey || !!apiKey };
    },
    async aiAsk(messages) {
      await new Promise((r) => setTimeout(r, 600));
      const last = messages[messages.length - 1]?.content ?? "";
      return `[mock] 浏览器预览模式没有真实 AI 后端。你问的是:「${last.slice(0, 60)}」\n\n在桌面应用的设置里配置 Claude API Key 后可用。示例命令:\n\`\`\`\nlsof -ti :8080 | xargs -r kill\n\`\`\``;
    },

    async restoreTargets() { return []; },
    async appQuit() { window.close(); },
    onCloseRequested: () => () => {},
  };
}
