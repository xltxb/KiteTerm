import { useEffect, useRef } from "react";
import { ipc } from "@/ipc";
import { Sidebar } from "./Sidebar";
import { CommandPalette } from "./CommandPalette";
import { TerminalScreen } from "@/screens/TerminalScreen";
import { HostsScreen } from "@/screens/HostsScreen";
import { TransferScreen } from "@/screens/TransferScreen";
import { ForwardsScreen } from "@/screens/ForwardsScreen";
import { MonitorScreen } from "@/screens/MonitorScreen";
import { KeysScreen } from "@/screens/KeysScreen";
import { SnippetsScreen } from "@/screens/SnippetsScreen";
import { SyncScreen } from "@/screens/SyncScreen";
import { SettingsScreen } from "@/screens/SettingsScreen";
import { useDataStore } from "@/stores/data";
import { useSessionStore } from "@/stores/session";
import { useSyncStore } from "@/stores/sync";
import { useUiStore } from "@/stores/ui";

export function App() {
  const { screen, setPalette, loadSettings, go } = useUiStore();
  const load = useDataStore((s) => s.load);
  const loadSync = useSyncStore((s) => s.load);

  const restored = useRef(false);

  useEffect(() => {
    void loadSettings();
    void load();
    void loadSync();

    // 启动恢复会话(设置关闭时后端返回空列表)
    if (!restored.current) {
      restored.current = true;
      void (async () => {
        const targets = await ipc.restoreTargets();
        for (const hostId of targets) {
          try {
            await useSessionStore.getState().open(hostId);
          } catch {
            // 需要密码或主机不可达的会话跳过,由用户手动重连
          }
        }
      })();
    }

    // 退出前确认(后端拦截 CloseRequested 后询问)
    return ipc.onCloseRequested((n) => {
      if (window.confirm(`还有 ${n} 个会话打开中,确认退出 KiteTerm?`)) {
        void ipc.appQuit();
      }
    });
  }, [loadSettings, load, loadSync]);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey;
      if (mod && (e.key === "k" || e.key === "K")) {
        e.preventDefault();
        setPalette(!useUiStore.getState().paletteOpen);
      } else if (e.key === "Escape" && useUiStore.getState().paletteOpen) {
        setPalette(false);
      } else if (mod && (e.key === "u" || e.key === "U")) {
        e.preventDefault();
        go("transfer");
      } else if (mod && e.key >= "1" && e.key <= "9") {
        const { sessions, setActive } = useSessionStore.getState();
        const idx = Number(e.key) - 1;
        if (sessions[idx]) {
          e.preventDefault();
          setActive(sessions[idx].id);
          go("terminal");
        }
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setPalette, go]);

  return (
    <div
      style={{
        display: "flex",
        height: "100vh",
        overflow: "hidden",
        fontSize: 14,
        background: "var(--surface-page)",
        color: "var(--text-body)",
        fontFamily: "var(--font-sans)",
      }}
    >
      <Sidebar />
      <main style={{ flex: 1, minWidth: 0, display: "flex", flexDirection: "column", overflow: "hidden" }}>
        {/* 终端界面始终挂载,避免切屏丢失 xterm DOM;其余界面按需渲染 */}
        <div style={{ display: screen === "terminal" ? "flex" : "none", flexDirection: "column", height: "100%", overflow: "hidden" }}>
          <TerminalScreen />
        </div>
        {screen === "hosts" && <HostsScreen />}
        {screen === "transfer" && <TransferScreen />}
        {screen === "forwards" && <ForwardsScreen />}
        {screen === "monitor" && <MonitorScreen />}
        {screen === "keys" && <KeysScreen />}
        {screen === "snippets" && <SnippetsScreen />}
        {screen === "sync" && <SyncScreen />}
        {screen === "settings" && <SettingsScreen />}
      </main>
      <CommandPalette />
    </div>
  );
}
