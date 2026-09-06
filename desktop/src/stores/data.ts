import { create } from "zustand";
import { ipc } from "@/ipc";
import type { ForwardRule, ForwardType, Host, HostGroup, Recording, Snippet, SshKey } from "@/types";

interface DataState {
  groups: HostGroup[];
  hosts: Host[];
  keys: SshKey[];
  snippets: Snippet[];
  forwards: ForwardRule[];
  recordings: Recording[];
  loaded: boolean;
  load: () => Promise<void>;
  createGroup: (name: string) => Promise<HostGroup>;
  renameGroup: (id: string, name: string) => Promise<void>;
  deleteGroup: (id: string) => Promise<void>;
  saveHost: (h: Partial<Host> & { name: string }) => Promise<void>;
  deleteHost: (id: string) => Promise<void>;
  generateKey: (name: string, algo: "ed25519" | "rsa4096") => Promise<void>;
  importKey: (name: string, pem: string) => Promise<void>;
  setKeySync: (id: string, enabled: boolean) => Promise<void>;
  deleteKey: (id: string) => Promise<void>;
  saveSnippet: (s: Partial<Snippet> & { name: string; command: string }) => Promise<void>;
  bumpSnippet: (id: string) => Promise<void>;
  deleteSnippet: (id: string) => Promise<void>;
  createForward: (rule: { type: ForwardType; localPort: number; target: string; viaHostId: string }) => Promise<void>;
  toggleForward: (id: string, enabled: boolean) => Promise<void>;
  deleteForward: (id: string) => Promise<void>;
  refreshRecordings: () => Promise<void>;
  setRecordingSync: (id: string, enabled: boolean) => Promise<void>;
}

export const useDataStore = create<DataState>((set, get) => ({
  groups: [],
  hosts: [],
  keys: [],
  snippets: [],
  forwards: [],
  recordings: [],
  loaded: false,
  load: async () => {
    const [groups, hosts, keys, snippets, forwards, recordings] = await Promise.all([
      ipc.listGroups(),
      ipc.listHosts(),
      ipc.listKeys(),
      ipc.listSnippets(),
      ipc.listForwards(),
      ipc.listRecordings(),
    ]);
    set({ groups, hosts, keys, snippets, forwards, recordings, loaded: true });
    ipc.onForwardStats((forwards) => set({ forwards }));
    ipc.onHostLatency((latency) =>
      set((s) => ({
        hosts: s.hosts.map((h) =>
          h.id in latency ? { ...h, latencyMs: latency[h.id] } : h,
        ),
      })),
    );
  },
  createGroup: async (name) => {
    const g = await ipc.createGroup(name);
    set({ groups: await ipc.listGroups() });
    return g;
  },
  renameGroup: async (id, name) => {
    await ipc.renameGroup(id, name);
    set({ groups: await ipc.listGroups() });
  },
  deleteGroup: async (id) => {
    await ipc.deleteGroup(id);
    // 组内主机会被迁移,分组与主机都要刷新
    const [groups, hosts] = await Promise.all([ipc.listGroups(), ipc.listHosts()]);
    set({ groups, hosts });
  },
  saveHost: async (h) => {
    await ipc.saveHost(h);
    set({ hosts: await ipc.listHosts() });
  },
  deleteHost: async (id) => {
    await ipc.deleteHost(id);
    set({ hosts: get().hosts.filter((x) => x.id !== id) });
  },
  generateKey: async (name, algo) => {
    await ipc.generateKey(name, algo);
    set({ keys: await ipc.listKeys() });
  },
  importKey: async (name, pem) => {
    await ipc.importKey(name, pem);
    set({ keys: await ipc.listKeys() });
  },
  setKeySync: async (id, enabled) => {
    await ipc.setKeySync(id, enabled);
    set({ keys: get().keys.map((k) => (k.id === id ? { ...k, syncEnabled: enabled } : k)) });
  },
  deleteKey: async (id) => {
    await ipc.deleteKey(id);
    set({ keys: get().keys.filter((k) => k.id !== id) });
  },
  saveSnippet: async (s) => {
    await ipc.saveSnippet(s);
    set({ snippets: await ipc.listSnippets() });
  },
  bumpSnippet: async (id) => {
    await ipc.bumpSnippet(id);
    set({ snippets: get().snippets.map((s) => (s.id === id ? { ...s, useCount: s.useCount + 1 } : s)) });
  },
  deleteSnippet: async (id) => {
    await ipc.deleteSnippet(id);
    set({ snippets: get().snippets.filter((s) => s.id !== id) });
  },
  createForward: async (rule) => {
    await ipc.createForward(rule);
    set({ forwards: await ipc.listForwards() });
  },
  toggleForward: async (id, enabled) => {
    set({ forwards: get().forwards.map((f) => (f.id === id ? { ...f, enabled } : f)) });
    await ipc.toggleForward(id, enabled);
    set({ forwards: await ipc.listForwards() });
  },
  deleteForward: async (id) => {
    await ipc.deleteForward(id);
    set({ forwards: get().forwards.filter((f) => f.id !== id) });
  },
  refreshRecordings: async () => {
    set({ recordings: await ipc.listRecordings() });
  },
  setRecordingSync: async (id, enabled) => {
    await ipc.setRecordingSync(id, enabled);
    set({ recordings: get().recordings.map((r) => (r.id === id ? { ...r, syncEnabled: enabled } : r)) });
  },
}));
