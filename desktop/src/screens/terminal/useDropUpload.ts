import { useEffect, useState, type RefObject } from "react";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { ipc } from "@/ipc";
import type { SessionInfo } from "@/types";

/** 拖文件到 SSH 终端画布 → 交给后端自动敲 rz 并以 ZMODEM 发送(仿 Xshell)。
 *
 *  Tauri 把系统级拖放事件发到整个窗口,这里按坐标判断是否落在 `surface` 上;
 *  返回值是「正悬停在画布上方」,用来渲染遮罩。只对已连接的 SSH 会话生效。 */
export function useDropUpload(surface: RefObject<HTMLElement | null>, session: SessionInfo | null): boolean {
  const [hovering, setHovering] = useState(false);
  const enabled = !!session && session.kind !== "local" && session.state === "connected";
  const sessionId = session?.id ?? null;

  useEffect(() => {
    if (!enabled || !sessionId) {
      setHovering(false);
      return;
    }
    const inside = (pos: { x: number; y: number }): boolean => {
      const el = surface.current;
      if (!el) return false;
      // 拖放坐标是物理像素,DOM 矩形是 CSS 像素
      const scale = window.devicePixelRatio || 1;
      const r = el.getBoundingClientRect();
      const x = pos.x / scale;
      const y = pos.y / scale;
      return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
    };
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    void getCurrentWebview()
      .onDragDropEvent((e) => {
        const p = e.payload;
        if (p.type === "leave") {
          setHovering(false);
        } else if (p.type === "enter" || p.type === "over") {
          setHovering(inside(p.position));
        } else if (p.type === "drop") {
          setHovering(false);
          if (p.paths.length > 0 && inside(p.position)) {
            void ipc.zmodemDropUpload(sessionId, p.paths);
          }
        }
      })
      .then((fn) => {
        if (cancelled) fn();
        else unlisten = fn;
      });
    return () => {
      cancelled = true;
      unlisten?.();
      setHovering(false);
    };
  }, [enabled, sessionId, surface]);

  return hovering;
}
