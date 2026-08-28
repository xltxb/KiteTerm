import { create } from "zustand";
import { ipc } from "@/ipc";
import type { MonitorSample } from "@/types";

interface MonitorState {
  samples: MonitorSample[];
  wired: boolean;
  load: () => Promise<void>;
}

export const useMonitorStore = create<MonitorState>((set, get) => ({
  samples: [],
  wired: false,
  load: async () => {
    if (!get().wired) {
      set({ wired: true });
      ipc.onMonitor((samples) => set({ samples }));
    }
    set({ samples: await ipc.monitorSamples() });
  },
}));
