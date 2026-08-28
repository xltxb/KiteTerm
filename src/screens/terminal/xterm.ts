import type { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import { SearchAddon } from "@xterm/addon-search";

/** 每个 xterm 实例的插件与挂载状态。挂在 WeakMap 上,终端销毁即随之回收。 */
export interface Addons {
  fit: FitAddon;
  search: SearchAddon;
  opened: boolean;
}

const addonMap = new WeakMap<Terminal, Addons>();

export function ensureAddons(term: Terminal): Addons {
  let a = addonMap.get(term);
  if (!a) {
    a = { fit: new FitAddon(), search: new SearchAddon(), opened: false };
    term.loadAddon(a.fit);
    term.loadAddon(a.search);
    addonMap.set(term, a);
  }
  return a;
}

export function getAddons(term: Terminal): Addons | undefined {
  return addonMap.get(term);
}
