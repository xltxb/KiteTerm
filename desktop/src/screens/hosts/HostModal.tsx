import { useState } from "react";
import { Check, Plus, X } from "lucide-react";
import { Button, TextInput } from "@/components/ui";
import { useDataStore } from "@/stores/data";
import type { Host } from "@/types";

/** 新建 / 编辑主机 */
export function HostModal({ edit, onClose }: { edit?: Host; onClose: () => void }) {
  const { groups, keys, saveHost, createGroup } = useDataStore();
  const [form, setForm] = useState(
    edit
      ? { name: edit.name, addr: edit.addr, port: String(edit.port), user: edit.user, groupId: edit.groupId, keyId: edit.keyId ?? "", tags: edit.tags.join(", ") }
      : { name: "", addr: "", port: "22", user: "root", groupId: groups[0]?.id ?? "", keyId: keys[0]?.id ?? "", tags: "" },
  );
  const set = (k: string, v: string) => setForm((f) => ({ ...f, [k]: v }));
  const valid = form.name.trim() && form.addr.trim();

  // 内联新建分组:创建后直接选中,避免为了建组中断填写流程
  const [creatingGroup, setCreatingGroup] = useState<string | null>(null);
  const addGroup = async () => {
    const name = (creatingGroup ?? "").trim();
    if (!name) return;
    const g = await createGroup(name);
    set("groupId", g.id);
    setCreatingGroup(null);
  };

  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3 className="modal-title">{edit ? "编辑主机" : "新建主机"}</h3>
        <div className="form-grid">
          <TextInput label="名称" placeholder="prod-web-03" value={form.name} onChange={(e) => set("name", e.target.value)} />
          <TextInput label="用户" placeholder="root" value={form.user} onChange={(e) => set("user", e.target.value)} />
          <TextInput label="地址" placeholder="10.24.0.33" value={form.addr} onChange={(e) => set("addr", e.target.value)} />
          <TextInput label="端口" placeholder="22" value={form.port} onChange={(e) => set("port", e.target.value)} />
          <div>
            <label className="input-label">分组</label>
            {creatingGroup === null ? (
              <div className="row gap-6">
                <select className="input grow" value={form.groupId} onChange={(e) => set("groupId", e.target.value)}>
                  {groups.map((g) => <option key={g.id} value={g.id}>{g.name}</option>)}
                </select>
                <div onClick={() => setCreatingGroup("")} title="新建分组" className="icon-btn">
                  <Plus size={15} />
                </div>
              </div>
            ) : (
              <div className="row gap-6">
                <input
                  className="input grow"
                  autoFocus
                  placeholder="新分组名称"
                  value={creatingGroup}
                  onChange={(e) => setCreatingGroup(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void addGroup();
                    if (e.key === "Escape") setCreatingGroup(null);
                  }}
                />
                <div onClick={() => void addGroup()} className="icon-btn" data-tone="accent">
                  <Check size={15} />
                </div>
                <div onClick={() => setCreatingGroup(null)} className="icon-btn">
                  <X size={15} />
                </div>
              </div>
            )}
          </div>
          <div>
            <label className="input-label">认证密钥</label>
            <select className="input w-full" value={form.keyId} onChange={(e) => set("keyId", e.target.value)}>
              {keys.map((k) => <option key={k.id} value={k.id}>{k.name}</option>)}
              <option value="">(密码认证)</option>
            </select>
          </div>
        </div>
        <div className="mt-12">
          <TextInput label="标签(逗号分隔)" placeholder="nginx, node" value={form.tags} onChange={(e) => set("tags", e.target.value)} />
        </div>
        <div className="modal-actions">
          <Button variant="ghost" onClick={onClose}>取消</Button>
          <Button
            variant="primary"
            disabled={!valid}
            onClick={async () => {
              await saveHost({
                id: edit?.id,
                name: form.name.trim(),
                addr: form.addr.trim(),
                port: Number(form.port) || 22,
                user: form.user.trim() || "root",
                groupId: form.groupId,
                keyId: form.keyId || null,
                tags: form.tags.split(",").map((t) => t.trim()).filter(Boolean),
              });
              onClose();
            }}
          >
            {edit ? "保存" : "创建"}
          </Button>
        </div>
      </div>
    </div>
  );
}
