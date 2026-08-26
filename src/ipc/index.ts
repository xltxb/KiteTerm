import type { VelaIpc } from "./api";
import { createTauriIpc } from "./tauri";

/**
 * Vela Term 是 macOS / Windows 桌面应用,全部能力经 Tauri IPC 由 Rust 核心提供,
 * 没有 Web 版本 —— 因此这里不做任何浏览器降级实现。
 */
export const ipc: VelaIpc = createTauriIpc();
export type { VelaIpc } from "./api";
