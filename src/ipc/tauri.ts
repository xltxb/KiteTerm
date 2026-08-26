// Tauri 环境下的真实 IPC 实现 — 命令名与 src-tauri/src/commands.rs 一一对应
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { ForwardRule, MonitorSample, SessionInfo, SyncStatus, TransferJob } from "@/types";
import type { Unsubscribe, VelaIpc } from "./api";

function sub<T>(event: string, cb: (payload: T) => void): Unsubscribe {
  const p = listen<T>(event, (e) => cb(e.payload));
  return () => { p.then((un) => un()); };
}

export function createTauriIpc(): VelaIpc {
  return {
    listGroups: () => invoke("list_groups"),
    listHosts: () => invoke("list_hosts"),
    saveHost: (host) => invoke("save_host", { host }),
    deleteHost: (id) => invoke("delete_host", { id }),

    listKeys: () => invoke("list_keys"),
    generateKey: (name, algo) => invoke("generate_key", { name, algo }),
    importKey: (name, pem) => invoke("import_key", { name, pem }),
    getPublicKey: (id) => invoke("get_public_key", { id }),
    setKeySync: (id, enabled) => invoke("set_key_sync", { id, enabled }),
    deleteKey: (id) => invoke("delete_key", { id }),

    listSnippets: () => invoke("list_snippets"),
    saveSnippet: (s) => invoke("save_snippet", { snippet: s }),
    bumpSnippet: (id) => invoke("bump_snippet", { id }),
    deleteSnippet: (id) => invoke("delete_snippet", { id }),

    listForwards: () => invoke("list_forwards"),
    createForward: (rule) => invoke("create_forward", { rule }),
    toggleForward: (id, enabled) => invoke("toggle_forward", { id, enabled }),
    deleteForward: (id) => invoke("delete_forward", { id }),
    onForwardStats: (cb) => sub<ForwardRule[]>("forward://stats", cb),

    openSession: (hostId, password) => invoke("session_open", { hostId, password }),
    trustHostKey: (hostId) => invoke("trust_host_key", { hostId }),
    writeSession: (id, data) => invoke("session_write", { id, data }),
    resizeSession: (id, cols, rows) => invoke("session_resize", { id, cols, rows }),
    closeSession: (id) => invoke("session_close", { id }),
    onSessionData: (cb) =>
      sub<{ id: string; data: string }>("term://data", (p) => {
        // 后端以 base64 发送保证二进制安全
        const bin = atob(p.data);
        const bytes = Uint8Array.from(bin, (c) => c.charCodeAt(0));
        cb(p.id, new TextDecoder("utf-8", { fatal: false }).decode(bytes));
      }),
    onSessionState: (cb) => sub<SessionInfo>("session://state", cb),

    sftpList: (sessionId, path) => invoke("sftp_list", { sessionId, path }),
    localList: (path) => invoke("local_list", { path }),
    localHome: () => invoke("local_home"),
    enqueueTransfer: (req) => invoke("transfer_enqueue", { req }),
    listTransfers: () => invoke("transfer_list"),
    retryTransfer: (id) => invoke("transfer_retry", { id }),
    onTransferProgress: (cb) => sub<TransferJob>("transfer://progress", cb),

    startRecording: (sessionId) => invoke("recording_start", { sessionId }),
    stopRecording: (sessionId) => invoke("recording_stop", { sessionId }),
    listRecordings: () => invoke("recording_list"),
    readRecording: (id) => invoke("recording_read", { id }),

    onHostLatency: (cb) => sub<Record<string, number | null>>("hosts://latency", cb),

    monitorSamples: () => invoke("monitor_samples"),
    onMonitor: (cb) => sub<MonitorSample[]>("monitor://samples", cb),

    getSyncStatus: () => invoke("sync_status"),
    getSyncConfig: () => invoke("sync_get_config"),
    saveSyncConfig: (cfg) => invoke("sync_save_config", { cfg }),
    syncNow: () => invoke("sync_now"),
    onSyncState: (cb) => sub<SyncStatus>("sync://state", cb),
    syncRecoveryPhrase: () => invoke("sync_recovery_phrase"),
    syncRestoreMaster: (phrase) => invoke("sync_restore_master", { phrase }),

    accountRegister: (email, password) => invoke("account_register", { email, password }),
    accountLogin: (email, password) => invoke("account_login", { email, password }),
    accountLogout: () => invoke("account_logout"),
    accountCurrent: () => invoke("account_current"),
    platformUrlGet: () => invoke("platform_url_get"),
    platformUrlSet: (url) => invoke("platform_url_set", { url }),

    getSettings: () => invoke("settings_get"),
    saveSettings: (s) => invoke("settings_save", { settings: s }),

    aiGetConfig: () => invoke("ai_get_config"),
    aiSaveConfig: (model, apiKey) => invoke("ai_save_config", { model, apiKey }),
    aiAsk: (messages, terminalContext) => invoke("ai_ask", { messages, terminalContext }),

    restoreTargets: () => invoke("restore_targets"),
    appQuit: () => invoke("app_quit"),
    onCloseRequested: (cb) => sub<number>("app://close-requested", cb),
  };
}
