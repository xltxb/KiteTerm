import { useEffect, useRef } from "react";
import { WebglAddon } from "@xterm/addon-webgl";
import { getTerminal } from "@/stores/session";
import { useUiStore } from "@/stores/ui";
import { ensureAddons, getAddons } from "./xterm";

/** xterm 的挂载点。终端实例由 session store 持有,这里只负责接上 DOM 与尺寸。 */
export function XtermPane({ sessionId }: { sessionId: string }) {
  const ref = useRef<HTMLDivElement>(null);
  const { settings } = useUiStore();

  useEffect(() => {
    const el = ref.current;
    const term = getTerminal(sessionId);
    if (!el || !term) return;
    const addons = ensureAddons(term);
    if (!addons.opened) {
      term.open(el);
      addons.opened = true;
      try {
        const webgl = new WebglAddon();
        webgl.onContextLoss(() => {
          console.error("[kite:term] WebGL context 丢失 —— 此后渲染退化为 DOM,终端会明显变卡");
        });
        term.loadAddon(webgl);
      } catch (e) {
        // xterm 5.x 已移除内置 canvas renderer,挂不上 WebGL 时退到的是 DOM
        // renderer(每字符一个 DOM 元素),会明显变卡 —— 所以这里不静默吞掉
        console.error("[kite:term] WebGL 不可用,已退回 DOM renderer:", e);
      }
    } else if (term.element && term.element.parentElement !== el) {
      el.appendChild(term.element);
    }
    addons.fit.fit();
    term.focus();
    const ro = new ResizeObserver(() => addons.fit.fit());
    ro.observe(el);
    return () => {
      ro.disconnect();
      // 切换会话时这个 div 会被 React 复用:不把旧实例的 DOM 摘掉,term.open 会把
      // 新画布追加到旧画布后面,旧的仍盖在上面 —— 表现为标签/状态栏是新会话、
      // 画面却还是上一个会话。实例本身留在注册表里,切回来时再挂回去,缓冲不丢。
      term.element?.remove();
    };
  }, [sessionId]);

  useEffect(() => {
    const term = getTerminal(sessionId);
    if (!term) return;
    term.options.fontSize = settings.terminalFontSize;
    term.options.cursorBlink = settings.cursorBlink;
    getAddons(term)?.fit.fit();
  }, [sessionId, settings.terminalFontSize, settings.cursorBlink]);

  return <div ref={ref} className="vterm term-canvas" />;
}
