import { isTauri, type VelaIpc } from "./api";
import { createMockIpc } from "./mock";
import { createTauriIpc } from "./tauri";

export const ipc: VelaIpc = isTauri() ? createTauriIpc() : createMockIpc();
export type { VelaIpc } from "./api";
