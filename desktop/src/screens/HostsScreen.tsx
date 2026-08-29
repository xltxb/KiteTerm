import { useMemo, useState } from "react";
import { Folders } from "lucide-react";
import { Badge, Button, ScreenHeader, TextInput } from "@/components/ui";
import { ipc } from "@/ipc";
import { GroupsModal } from "@/screens/hosts/GroupsModal";
import { HostModal } from "@/screens/hosts/HostModal";
import { HostRow } from "@/screens/hosts/HostRow";
import { PasswordModal } from "@/screens/hosts/PasswordModal";
import { useDataStore } from "@/stores/data";
import { useSessionStore } from "@/stores/session";
import { useUiStore } from "@/stores/ui";
import type { Host } from "@/types";
import { confirmAction, showError } from "@/lib/dialog";

export function HostsScreen() {
  const { groups, hosts, deleteHost } = useDataStore();
  const keys = useDataStore((s) => s.keys);
  const open = useSessionStore((s) => s.open);
  const go = useUiStore((s) => s.go);
  const [query, setQuery] = useState("");
  const [showNew, setShowNew] = useState(false);
  const [showGroups, setShowGroups] = useState(false);
  const [editHost, setEditHost] = useState<Host | null>(null);
  const [connecting, setConnecting] = useState<string | null>(null);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return hosts;
    return hosts.filter((h) => (h.name + " " + h.addr + " " + h.user + " " + h.tags.join(" ")).toLowerCase().includes(q));
  }, [hosts, query]);

  const keyName = (id: string | null) => keys.find((k) => k.id === id)?.name ?? "密码认证";

  const [pwHost, setPwHost] = useState<Host | null>(null);
  const [pwError, setPwError] = useState<string | null>(null);

  const connect = async (h: Host, password?: string) => {
    setConnecting(h.id);
    try {
      await open(h.id, password);
      setPwHost(null);
      setPwError(null);
      go("terminal");
    } catch (err) {
      const msg = String(err);
      if (msg.includes("PASSWORD_REQUIRED")) {
        setPwHost(h);
        setPwError(null);
      } else if (msg.includes("HOSTKEY_CHANGED")) {
        const detail = msg.replace(/^.*HOSTKEY_CHANGED:\s*/, "");
        if (await confirmAction(detail + "\n\n信任新指纹并重试连接?", "主机指纹已变更")) {
          await ipc.trustHostKey(h.id);
          setConnecting(null);
          return connect(h, password);
        }
      } else if (password !== undefined) {
        setPwError(msg);
      } else {
        void showError(msg, "连接失败");
      }
    } finally {
      setConnecting(null);
    }
  };

  return (
    <div className="screen vscroll">
      <div className="screen-inner wide">
        <ScreenHeader eyebrow="HOSTS" title="主机" count={String(hosts.length)}>
          <TextInput className="search" placeholder="搜索主机、标签…" value={query} onChange={(e) => setQuery(e.target.value)} />
          <Button variant="outline" onClick={() => setShowGroups(true)}>
            <Folders size={14} />管理分组
          </Button>
          <Button variant="primary" onClick={() => setShowNew(true)}>新建主机</Button>
        </ScreenHeader>

        {groups.map((g) => {
          const ghosts = filtered.filter((h) => h.groupId === g.id);
          if (ghosts.length === 0) return null;
          return (
            <div key={g.id} className="mb-20">
              <div className="fs-sm w-bold t-muted tracking-wide mb-8">{g.name}</div>
              <div className="card clip">
                {ghosts.map((h) => (
                  <HostRow
                    key={h.id}
                    host={h}
                    keyName={keyName(h.keyId)}
                    connecting={connecting === h.id}
                    onConnect={() => void connect(h)}
                    onEdit={() => setEditHost(h)}
                    onDelete={async () => { if (await confirmAction(`删除主机 ${h.name}?`, "删除主机")) await deleteHost(h.id); }}
                  />
                ))}
              </div>
            </div>
          );
        })}

        {filtered.length === 0 && (
          <div className="card empty-state boxed fs-md t-faint">
            没有匹配的主机 {query && <Badge color="neutral" className="ml-6">{query}</Badge>}
          </div>
        )}
      </div>
      {showNew && <HostModal onClose={() => setShowNew(false)} />}
      {showGroups && <GroupsModal onClose={() => setShowGroups(false)} />}
      {editHost && <HostModal edit={editHost} onClose={() => setEditHost(null)} />}
      {pwHost && (
        <PasswordModal
          host={pwHost}
          error={pwError}
          busy={connecting === pwHost.id}
          onSubmit={(pw) => void connect(pwHost, pw)}
          onClose={() => { setPwHost(null); setPwError(null); }}
        />
      )}
    </div>
  );
}
