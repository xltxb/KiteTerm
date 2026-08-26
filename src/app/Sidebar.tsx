import {
  Activity,
  ArrowDownUp,
  ArrowRightLeft,
  Braces,
  Cloud,
  Command,
  KeyRound,
  Moon,
  PanelLeftClose,
  PanelLeftOpen,
  Sailboat,
  Server,
  Settings,
  SquareTerminal,
  Sun,
} from "lucide-react";
import type { ReactNode } from "react";
import { isTauri } from "@/ipc/api";
import { useSyncStore } from "@/stores/sync";
import { useUiStore } from "@/stores/ui";
import type { Screen } from "@/types";

const NAV: { screen: Screen; label: string; icon: ReactNode }[] = [
  { screen: "terminal", label: "终端", icon: <SquareTerminal size={18} /> },
  { screen: "hosts", label: "主机", icon: <Server size={18} /> },
  { screen: "transfer", label: "传输", icon: <ArrowDownUp size={18} /> },
  { screen: "forwards", label: "转发", icon: <ArrowRightLeft size={18} /> },
  { screen: "monitor", label: "监控", icon: <Activity size={18} /> },
  { screen: "keys", label: "密钥", icon: <KeyRound size={18} /> },
  { screen: "snippets", label: "片段", icon: <Braces size={18} /> },
  { screen: "sync", label: "同步", icon: <Cloud size={18} /> },
  { screen: "settings", label: "设置", icon: <Settings size={18} /> },
];

function NavItem({
  active,
  collapsed,
  icon,
  label,
  trailing,
  onClick,
  small,
}: {
  active?: boolean;
  collapsed: boolean;
  icon: ReactNode;
  label: string;
  trailing?: ReactNode;
  onClick: () => void;
  small?: boolean;
}) {
  return (
    <div
      onClick={onClick}
      className={active ? undefined : "hover-row"}
      style={{
        display: "flex",
        alignItems: "center",
        justifyContent: collapsed ? "center" : "flex-start",
        gap: small ? 10 : 11,
        padding: collapsed ? "9px 0" : "9px 12px",
        borderRadius: 10,
        cursor: "pointer",
        fontWeight: 600,
        fontSize: small ? 12.5 : 13.5,
        color: active ? "var(--accent-text)" : "var(--text-muted)",
        background: active ? "var(--accent-subtle)" : "transparent",
      }}
    >
      {icon}
      {!collapsed && <span style={{ flex: trailing ? 1 : undefined }}>{label}</span>}
      {!collapsed && trailing}
    </div>
  );
}

export function Sidebar() {
  const { screen, go, sidebarCollapsed, toggleSidebar, setPalette, settings, setSettings } = useUiStore();
  const syncStatus = useSyncStore((s) => s.status);
  const collapsed = sidebarCollapsed;
  const dark = settings.theme === "dark";

  return (
    <aside
      className="vscroll"
      style={{
        width: collapsed ? 64 : 228,
        flex: "none",
        display: "flex",
        flexDirection: "column",
        gap: 4,
        padding: "16px 12px",
        borderRight: "1px solid var(--border-subtle)",
        background: "var(--surface-card)",
        overflowY: "auto",
        overflowX: "hidden",
      }}
    >
      {/* Tauri(macOS Overlay 标题栏)下原生红绿灯会叠加于此处,只留拖拽区;浏览器预览时画假按钮 */}
      <div style={{ display: "flex", gap: 8, padding: "2px 6px 14px 6px", minHeight: 26 }} data-tauri-drag-region>
        {!isTauri() && (
          <>
            <span style={{ width: 12, height: 12, borderRadius: 99, background: "#ff5f57" }} />
            <span style={{ width: 12, height: 12, borderRadius: 99, background: "#febc2e" }} />
            <span style={{ width: 12, height: 12, borderRadius: 99, background: "#28c840" }} />
          </>
        )}
      </div>
      <div style={{ display: "flex", alignItems: "center", gap: 10, padding: "4px 6px 14px 6px" }}>
        <div
          className="icon-chip"
          style={{
            width: 34,
            height: 34,
            background: "linear-gradient(135deg,var(--azure-500),var(--cyan-400))",
            color: "#fff",
            boxShadow: "var(--glow-sm)",
          }}
        >
          <Sailboat size={18} />
        </div>
        {!collapsed && (
          <div style={{ display: "flex", flexDirection: "column", lineHeight: 1.15 }}>
            <span style={{ fontFamily: "var(--font-display)", fontWeight: 700, fontSize: 16, color: "var(--text-strong)", letterSpacing: "-0.02em" }}>
              Vela Term
            </span>
            <span style={{ fontFamily: "var(--font-mono)", fontSize: 10, letterSpacing: "0.09em", color: "var(--text-faint)" }}>
              SSH · SCP · SYNC
            </span>
          </div>
        )}
      </div>

      {NAV.map((n) => (
        <NavItem
          key={n.screen}
          active={screen === n.screen}
          collapsed={collapsed}
          icon={n.icon}
          label={n.label}
          onClick={() => go(n.screen)}
        />
      ))}

      <div style={{ flex: 1 }} />

      <NavItem
        collapsed={collapsed}
        small
        icon={collapsed ? <PanelLeftOpen size={15} /> : <PanelLeftClose size={15} />}
        label="收起侧栏"
        onClick={toggleSidebar}
      />
      <NavItem
        collapsed={collapsed}
        small
        icon={
          <span
            style={{
              width: 7,
              height: 7,
              flex: "none",
              borderRadius: 99,
              background: syncStatus.syncing ? "var(--warning)" : "var(--success)",
            }}
          />
        }
        label={syncStatus.syncing ? "同步中…" : `已同步 · ${syncStatus.lastSyncAt ?? "从未"}`}
        onClick={() => go("sync")}
      />
      <NavItem
        collapsed={collapsed}
        small
        icon={<Command size={15} />}
        label="命令面板"
        trailing={<span className="kbd" style={{ fontSize: 10.5, padding: "2px 6px" }}>⌘K</span>}
        onClick={() => setPalette(true)}
      />
      <NavItem
        collapsed={collapsed}
        small
        icon={dark ? <Sun size={15} /> : <Moon size={15} />}
        label={dark ? "切换浅色" : "切换深色"}
        onClick={() => setSettings({ theme: dark ? "light" : "dark" })}
      />

      <div
        style={{
          borderTop: "1px solid var(--border-subtle)",
          margin: "8px 4px 4px 4px",
          paddingTop: 12,
          display: "flex",
          alignItems: "center",
          justifyContent: collapsed ? "center" : "flex-start",
          gap: 10,
        }}
      >
        <div
          style={{
            width: 30,
            height: 30,
            flex: "none",
            borderRadius: 99,
            background: "var(--accent-subtle)",
            color: "var(--accent-text)",
            display: "flex",
            alignItems: "center",
            justifyContent: "center",
            fontWeight: 700,
            fontSize: 13,
          }}
        >
          L
        </div>
        {!collapsed && (
          <div style={{ display: "flex", flexDirection: "column", lineHeight: 1.2 }}>
            <span style={{ fontWeight: 600, fontSize: 13, color: "var(--text-strong)" }}>Lin</span>
            <span style={{ fontSize: 11, color: "var(--text-faint)" }}>Pro 工作区</span>
          </div>
        )}
      </div>
    </aside>
  );
}
