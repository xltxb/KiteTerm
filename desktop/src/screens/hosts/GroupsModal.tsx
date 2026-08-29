import { useState } from "react";
import { Check, Pencil, Trash2, X } from "lucide-react";
import { Button } from "@/components/ui";
import { useDataStore } from "@/stores/data";
import { confirmAction } from "@/lib/dialog";

/** 分组管理:新建 / 重命名 / 删除(删除时组内主机迁到其他分组) */
export function GroupsModal({ onClose }: { onClose: () => void }) {
  const { groups, hosts, createGroup, renameGroup, deleteGroup } = useDataStore();
  const [newName, setNewName] = useState("");
  const [editing, setEditing] = useState<{ id: string; name: string } | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const hostCount = (gid: string) => hosts.filter((h) => h.groupId === gid).length;

  const run = async (fn: () => Promise<unknown>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(String(e).replace(/^.*AppError\("?|"?\)$/g, ""));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal md" onClick={(e) => e.stopPropagation()}>
        <h3 className="modal-title tight">管理分组</h3>
        <div className="fs-sm t-muted lh-relaxed mb-16">
          分组用于组织主机列表,会随「主机配置」一并云同步。删除分组时,组内主机将迁移到第一个分组而不会被删除。
        </div>

        <div className="col gap-6 mb-16">
          {groups.map((g) => (
            <div
              key={g.id}
              className="hover-row row gap-10 r-md bd-subtle p-8"
            >
              {editing?.id === g.id ? (
                <>
                  <input
                    className="input sm grow"
                    autoFocus
                    value={editing.name}
                    onChange={(e) => setEditing({ id: g.id, name: e.target.value })}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && editing.name.trim()) {
                        void run(async () => { await renameGroup(g.id, editing.name.trim()); setEditing(null); });
                      }
                      if (e.key === "Escape") setEditing(null);
                    }}
                  />
                  <div
                    onClick={() => editing.name.trim() && void run(async () => { await renameGroup(g.id, editing.name.trim()); setEditing(null); })}
                    className="icon-btn sm"
                    data-tone="accent"
                  >
                    <Check size={15} />
                  </div>
                  <div onClick={() => setEditing(null)} className="icon-btn sm">
                    <X size={15} />
                  </div>
                </>
              ) : (
                <>
                  <span className="grow fs-md w-semi t-strong">{g.name}</span>
                  <span className="fs-xs t-faint">{hostCount(g.id)} 台主机</span>
                  <div
                    onClick={() => setEditing({ id: g.id, name: g.name })}
                    title="重命名"
                    className="icon-btn sm plain"
                  >
                    <Pencil size={14} />
                  </div>
                  <div
                    onClick={async () => {
                      const n = hostCount(g.id);
                      const msg = n > 0
                        ? `删除分组「${g.name}」?组内 ${n} 台主机会迁移到其他分组(不会被删除)。`
                        : `删除分组「${g.name}」?`;
                      if (await confirmAction(msg, "删除分组")) void run(() => deleteGroup(g.id));
                    }}
                    title="删除"
                    className="icon-btn sm plain"
                    style={{ opacity: groups.length <= 1 ? 0.35 : 1, pointerEvents: groups.length <= 1 ? "none" : "auto" }}
                  >
                    <Trash2 size={14} />
                  </div>
                </>
              )}
            </div>
          ))}
        </div>

        <div className="row gap-8">
          <input
            className="input grow"
            placeholder="新分组名称"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && newName.trim()) {
                void run(async () => { await createGroup(newName.trim()); setNewName(""); });
              }
            }}
          />
          <Button
            variant="primary"
            loading={busy}
            disabled={!newName.trim()}
            onClick={() => void run(async () => { await createGroup(newName.trim()); setNewName(""); })}
          >
            新建分组
          </Button>
        </div>

        {error && <div className="fs-sm t-danger mt-12 lh-normal">{error}</div>}

        <div className="modal-actions">
          <Button variant="ghost" onClick={onClose}>完成</Button>
        </div>
      </div>
    </div>
  );
}
