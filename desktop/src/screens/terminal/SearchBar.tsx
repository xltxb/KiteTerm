import { useEffect, useRef, useState } from "react";
import { Search, X } from "lucide-react";
import { getTerminal } from "@/stores/session";
import { ensureAddons } from "./xterm";

/** ⌘F 唤起的输出搜索条,浮在终端右上角。 */
export function SearchBar({ sessionId, onClose }: { sessionId: string; onClose: () => void }) {
  const [q, setQ] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  useEffect(() => inputRef.current?.focus(), []);
  const term = getTerminal(sessionId);
  const search = term ? ensureAddons(term).search : null;

  return (
    <div className="search-pop">
      <Search size={14} />
      <input
        ref={inputRef}
        value={q}
        onChange={(e) => {
          setQ(e.target.value);
          search?.findNext(e.target.value);
        }}
        onKeyDown={(e) => {
          if (e.key === "Enter") search?.findNext(q);
          if (e.key === "Escape") onClose();
        }}
        placeholder="搜索输出…"
      />
      <X size={14} className="clickable" onClick={onClose} />
    </div>
  );
}
