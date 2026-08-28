import { Moon, PanelLeftClose, PanelLeftOpen, Sun } from "lucide-react";
import type { ReactNode } from "react";
import { KiteMark } from "@/components/KiteMark";
import {
  KtCommand,
  KtForward,
  KtHosts,
  KtKeys,
  KtMonitor,
  KtSettings,
  KtSnippets,
  KtSync,
  KtTerminal,
  KtTransfer,
} from "@/components/icons";
import { useFeaturesStore } from "@/stores/features";
import { useSyncStore } from "@/stores/sync";
import { useUiStore } from "@/stores/ui";
import type { Screen } from "@/types";

const NAV: { screen: Screen; label: string; icon: ReactNode }[] = [
  { screen: "terminal", label: "终端", icon: <KtTerminal size={18} /> },
  { screen: "hosts", label: "主机", icon: <KtHosts size={18} /> },
  { screen: "transfer", label: "传输", icon: <KtTransfer size={18} /> },
  { screen: "forwards", label: "转发", icon: <KtForward size={18} /> },
  { screen: "monitor", label: "监控", icon: <KtMonitor size={18} /> },
  { screen: "keys", label: "密钥", icon: <KtKeys size={18} /> },
  { screen: "snippets", label: "片段", icon: <KtSnippets size={18} /> },
  { screen: "sync", label: "同步", icon: <KtSync size={18} /> },
  { screen: "settings", label: "设置", icon: <KtSettings size={18} /> },
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
      title={collapsed ? label : undefined}
      className={small ? "nav-item sm" : "nav-item"}
      data-active={active ? "true" : "false"}
    >
      {icon}
      {!collapsed && <span className={trailing ? "grow" : undefined}>{label}</span>}
      {!collapsed && trailing}
    </div>
  );
}

export function Sidebar() {
  const { screen, go, sidebarCollapsed, toggleSidebar, setPalette, settings, setSettings } = useUiStore();
  const syncStatus = useSyncStore((s) => s.status);
  const features = useFeaturesStore((s) => s.features);
  const collapsed = sidebarCollapsed;
  const dark = settings.theme === "dark";

  return (
    <aside className="sidebar vscroll" data-collapsed={collapsed ? "true" : "false"}>
      {/* macOS Overlay 标题栏的原生窗口按钮叠加于此处;Windows 下为纯拖拽区 */}
      <div style={{ padding: "2px 6px 14px 6px", minHeight: 26 }} data-tauri-drag-region />

      <div className="row gap-10" style={{ padding: "4px 6px 14px 6px" }}>
        <div className="brand-mark">
          <KiteMark size={18} />
        </div>
        {!collapsed && (
          <div className="col lh-tight">
            <span className="brand-name">KiteTerm</span>
            <span className="brand-sub">SSH · SCP · SYNC</span>
          </div>
        )}
      </div>

      {NAV.filter((n) => n.screen !== "sync" || features.sync).map((n) => (
        <NavItem
          key={n.screen}
          active={screen === n.screen}
          collapsed={collapsed}
          icon={n.icon}
          label={n.label}
          onClick={() => go(n.screen)}
        />
      ))}

      <div className="spacer" />

      <NavItem
        collapsed={collapsed}
        small
        icon={collapsed ? <PanelLeftOpen size={15} /> : <PanelLeftClose size={15} />}
        label={collapsed ? "展开侧栏" : "收起侧栏"}
        onClick={toggleSidebar}
      />
      {features.sync && (
        <NavItem
          collapsed={collapsed}
          small
          icon={<span className={syncStatus.syncing ? "dot-lg dot-warn" : "dot-lg dot-ok"} />}
          label={syncStatus.syncing ? "同步中…" : `已同步 · ${syncStatus.lastSyncAt ?? "从未"}`}
          onClick={() => go("sync")}
        />
      )}
      <NavItem
        collapsed={collapsed}
        small
        icon={<KtCommand size={15} />}
        label="命令面板"
        trailing={<span className="kbd sm">⌘K</span>}
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
        className={collapsed ? "row jcenter gap-10 bd-top" : "row gap-10 bd-top"}
        style={{ margin: "8px 4px 4px 4px", paddingTop: 12 }}
      >
        <div className="avatar">L</div>
        {!collapsed && (
          <div className="col lh-tight">
            <span className="fs-md w-semi t-strong">Lin</span>
            <span className="fs-xs t-faint">Pro 工作区</span>
          </div>
        )}
      </div>
    </aside>
  );
}
