import { useMemo, useState } from "react";
import { Check, Cpu, Database, Folders, Globe, KeyRound, Lock, Pencil, Plus, Server, Trash2, X } from "lucide-react";
import { Badge, Button, ScreenHeader, TextInput } from "@/components/ui";
import { ipc } from "@/ipc";
import { useDataStore } from "@/stores/data";
import { useSessionStore } from "@/stores/session";
import { useUiStore } from "@/stores/ui";
import type { Host } from "@/types";

function hostIcon(h: Host) {
  const tags = h.tags.join(" ");
  if (/postgres|redis|mysql|db/.test(tags) || h.name.includes("db") || h.name.includes("redis")) return <Database size={17} />;
  if (h.name.includes("pi")) return <Cpu size={17} />;
  if (h.name.includes("vps")) return <Globe size={17} />;
  return <Server size={17} />;
}

function latColor(ms: number | null) {
  if (ms == null) return "var(--text-faint)";
  if (ms < 50) return "var(--success)";
  if (ms < 120) return "var(--warning)";
  return "var(--danger)";
}

/** 分组管理:新建 / 重命名 / 删除(删除时组内主机迁到其他分组) */
function GroupsModal({ onClose }: { onClose: () => void }) {
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
      <div className="modal" onClick={(e) => e.stopPropagation()} style={{ width: 460 }}>
        <h3 style={{ fontSize: 17, marginBottom: 6 }}>管理分组</h3>
        <div style={{ fontSize: 12.5, color: "var(--text-muted)", lineHeight: 1.6, marginBottom: 14 }}>
          分组用于组织主机列表,会随「主机配置」一并云同步。删除分组时,组内主机将迁移到第一个分组而不会被删除。
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 6, marginBottom: 14 }}>
          {groups.map((g) => (
            <div
              key={g.id}
              className="hover-row"
              style={{ display: "flex", alignItems: "center", gap: 10, padding: "8px 10px", borderRadius: 10, border: "1px solid var(--border-subtle)" }}
            >
              {editing?.id === g.id ? (
                <>
                  <input
                    className="input"
                    autoFocus
                    value={editing.name}
                    onChange={(e) => setEditing({ id: g.id, name: e.target.value })}
                    onKeyDown={(e) => {
                      if (e.key === "Enter" && editing.name.trim()) {
                        void run(async () => { await renameGroup(g.id, editing.name.trim()); setEditing(null); });
                      }
                      if (e.key === "Escape") setEditing(null);
                    }}
                    style={{ flex: 1, height: 30 }}
                  />
                  <div
                    onClick={() => editing.name.trim() && void run(async () => { await renameGroup(g.id, editing.name.trim()); setEditing(null); })}
                    style={{ padding: 5, borderRadius: 7, cursor: "pointer", color: "var(--accent-text)" }}
                  >
                    <Check size={15} />
                  </div>
                  <div onClick={() => setEditing(null)} style={{ padding: 5, borderRadius: 7, cursor: "pointer", color: "var(--text-faint)" }}>
                    <X size={15} />
                  </div>
                </>
              ) : (
                <>
                  <span style={{ flex: 1, fontSize: 13.5, fontWeight: 600, color: "var(--text-strong)" }}>{g.name}</span>
                  <span style={{ fontSize: 11.5, color: "var(--text-faint)" }}>{hostCount(g.id)} 台主机</span>
                  <div
                    onClick={() => setEditing({ id: g.id, name: g.name })}
                    title="重命名"
                    style={{ padding: 5, borderRadius: 7, cursor: "pointer", color: "var(--text-faint)" }}
                  >
                    <Pencil size={14} />
                  </div>
                  <div
                    onClick={() => {
                      const n = hostCount(g.id);
                      const msg = n > 0
                        ? `删除分组「${g.name}」?组内 ${n} 台主机会迁移到其他分组(不会被删除)。`
                        : `删除分组「${g.name}」?`;
                      if (confirm(msg)) void run(() => deleteGroup(g.id));
                    }}
                    title="删除"
                    style={{ padding: 5, borderRadius: 7, cursor: "pointer", color: "var(--text-faint)", opacity: groups.length <= 1 ? 0.35 : 1, pointerEvents: groups.length <= 1 ? "none" : "auto" }}
                  >
                    <Trash2 size={14} />
                  </div>
                </>
              )}
            </div>
          ))}
        </div>

        <div style={{ display: "flex", gap: 8 }}>
          <input
            className="input"
            placeholder="新分组名称"
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && newName.trim()) {
                void run(async () => { await createGroup(newName.trim()); setNewName(""); });
              }
            }}
            style={{ flex: 1 }}
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

        {error && <div style={{ fontSize: 12, color: "var(--danger-text)", marginTop: 10, lineHeight: 1.5 }}>{error}</div>}

        <div style={{ display: "flex", justifyContent: "flex-end", marginTop: 18 }}>
          <Button variant="ghost" onClick={onClose}>完成</Button>
        </div>
      </div>
    </div>
  );
}

function HostModal({ edit, onClose }: { edit?: Host; onClose: () => void }) {
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
        <h3 style={{ fontSize: 17, marginBottom: 16 }}>{edit ? "编辑主机" : "新建主机"}</h3>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 12 }}>
          <TextInput label="名称" placeholder="prod-web-03" value={form.name} onChange={(e) => set("name", e.target.value)} />
          <TextInput label="用户" placeholder="root" value={form.user} onChange={(e) => set("user", e.target.value)} />
          <TextInput label="地址" placeholder="10.24.0.33" value={form.addr} onChange={(e) => set("addr", e.target.value)} />
          <TextInput label="端口" placeholder="22" value={form.port} onChange={(e) => set("port", e.target.value)} />
          <div>
            <label className="input-label">分组</label>
            {creatingGroup === null ? (
              <div style={{ display: "flex", gap: 6 }}>
                <select className="input" style={{ flex: 1, minWidth: 0 }} value={form.groupId} onChange={(e) => set("groupId", e.target.value)}>
                  {groups.map((g) => <option key={g.id} value={g.id}>{g.name}</option>)}
                </select>
                <div
                  onClick={() => setCreatingGroup("")}
                  title="新建分组"
                  className="hover-row"
                  style={{ width: 36, height: 36, flex: "none", borderRadius: 10, border: "1px solid var(--border-default)", display: "flex", alignItems: "center", justifyContent: "center", cursor: "pointer", color: "var(--text-muted)" }}
                >
                  <Plus size={15} />
                </div>
              </div>
            ) : (
              <div style={{ display: "flex", gap: 6 }}>
                <input
                  className="input"
                  autoFocus
                  placeholder="新分组名称"
                  value={creatingGroup}
                  onChange={(e) => setCreatingGroup(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") void addGroup();
                    if (e.key === "Escape") setCreatingGroup(null);
                  }}
                  style={{ flex: 1, minWidth: 0 }}
                />
                <div
                  onClick={() => void addGroup()}
                  className="hover-row"
                  style={{ width: 36, height: 36, flex: "none", borderRadius: 10, border: "1px solid var(--border-default)", display: "flex", alignItems: "center", justifyContent: "center", cursor: "pointer", color: "var(--accent-text)" }}
                >
                  <Check size={15} />
                </div>
                <div
                  onClick={() => setCreatingGroup(null)}
                  className="hover-row"
                  style={{ width: 36, height: 36, flex: "none", borderRadius: 10, border: "1px solid var(--border-default)", display: "flex", alignItems: "center", justifyContent: "center", cursor: "pointer", color: "var(--text-faint)" }}
                >
                  <X size={15} />
                </div>
              </div>
            )}
          </div>
          <div>
            <label className="input-label">认证密钥</label>
            <select className="input" style={{ width: "100%" }} value={form.keyId} onChange={(e) => set("keyId", e.target.value)}>
              {keys.map((k) => <option key={k.id} value={k.id}>{k.name}</option>)}
              <option value="">(密码认证)</option>
            </select>
          </div>
        </div>
        <div style={{ marginTop: 12 }}>
          <TextInput label="标签(逗号分隔)" placeholder="nginx, node" value={form.tags} onChange={(e) => set("tags", e.target.value)} style={{ width: "100%" }} />
        </div>
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 20 }}>
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
        if (window.confirm(detail + "\n\n信任新指纹并重试连接?")) {
          await ipc.trustHostKey(h.id);
          setConnecting(null);
          return connect(h, password);
        }
      } else if (password !== undefined) {
        setPwError(msg);
      } else {
        alert("连接失败: " + msg);
      }
    } finally {
      setConnecting(null);
    }
  };

  return (
    <div className="screen vscroll">
      <div className="screen-inner" style={{ maxWidth: 960 }}>
        <ScreenHeader eyebrow="HOSTS" title="主机" count={String(hosts.length)}>
          <TextInput placeholder="搜索主机、标签…" value={query} onChange={(e) => setQuery(e.target.value)} style={{ width: 220 }} />
          <Button variant="outline" onClick={() => setShowGroups(true)}>
            <Folders size={14} />管理分组
          </Button>
          <Button variant="primary" onClick={() => setShowNew(true)}>新建主机</Button>
        </ScreenHeader>

        {groups.map((g) => {
          const ghosts = filtered.filter((h) => h.groupId === g.id);
          if (ghosts.length === 0) return null;
          return (
            <div key={g.id} style={{ marginBottom: 20 }}>
              <div style={{ fontSize: 12, fontWeight: 700, letterSpacing: "0.02em", color: "var(--text-muted)", margin: "0 2px 8px 2px" }}>{g.name}</div>
              <div className="card" style={{ overflow: "hidden" }}>
                {ghosts.map((h) => (
                  <div key={h.id} className="hover-row" style={{ display: "flex", alignItems: "center", gap: 14, padding: "13px 18px", borderBottom: "1px solid var(--border-subtle)" }}>
                    <div className="icon-chip" style={{ width: 36, height: 36 }}>{hostIcon(h)}</div>
                    <div style={{ width: 200, flex: "none", lineHeight: 1.3 }}>
                      <div style={{ fontWeight: 600, fontSize: 13.5, color: "var(--text-strong)" }}>{h.name}</div>
                      <div style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--text-faint)" }}>
                        {h.user}@{h.addr} · :{h.port}
                      </div>
                    </div>
                    <div style={{ display: "flex", gap: 6, flex: 1, minWidth: 0, overflow: "hidden" }}>
                      {h.tags.map((t) => <span key={t} className="tag-pill">{t}</span>)}
                    </div>
                    <span style={{ display: "flex", alignItems: "center", gap: 6, fontFamily: "var(--font-mono)", fontSize: 11.5, color: "var(--text-muted)", width: 64 }}>
                      <span style={{ width: 6, height: 6, borderRadius: 99, background: latColor(h.latencyMs) }} />
                      {h.latencyMs != null ? `${h.latencyMs}ms` : "—"}
                    </span>
                    <span style={{ display: "flex", alignItems: "center", gap: 5, fontSize: 11.5, color: "var(--text-faint)", width: 150 }}>
                      <KeyRound size={12} />{keyName(h.keyId)}
                    </span>
                    <span style={{ fontSize: 11.5, color: "var(--text-faint)", width: 96 }}>{h.lastConnectedAt ?? "从未连接"}</span>
                    <Button variant="secondary" loading={connecting === h.id} onClick={() => void connect(h)}>连接</Button>
                    <div
                      className="hover-row"
                      onClick={() => setEditHost(h)}
                      title="编辑"
                      style={{ padding: 6, borderRadius: 8, cursor: "pointer", color: "var(--text-faint)" }}
                    >
                      <Pencil size={15} />
                    </div>
                    <div
                      className="hover-row"
                      onClick={() => { if (confirm(`删除主机 ${h.name}?`)) void deleteHost(h.id); }}
                      style={{ padding: 6, borderRadius: 8, cursor: "pointer", color: "var(--text-faint)" }}
                    >
                      <Trash2 size={15} />
                    </div>
                  </div>
                ))}
              </div>
            </div>
          );
        })}

        {filtered.length === 0 && (
          <div className="card" style={{ padding: "36px 20px", textAlign: "center", color: "var(--text-faint)", fontSize: 13 }}>
            没有匹配的主机 {query && <Badge color="neutral" style={{ marginLeft: 6 }}>{query}</Badge>}
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

function PasswordModal({
  host,
  error,
  busy,
  onSubmit,
  onClose,
}: {
  host: Host;
  error: string | null;
  busy: boolean;
  onSubmit: (password: string) => void;
  onClose: () => void;
}) {
  const [pw, setPw] = useState("");
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()} style={{ width: 380 }}>
        <h3 style={{ fontSize: 16, marginBottom: 6, display: "flex", alignItems: "center", gap: 8 }}>
          <Lock size={15} />密码认证
        </h3>
        <div style={{ fontSize: 12.5, color: "var(--text-muted)", marginBottom: 14 }}>
          {host.user}@{host.addr}:{host.port} 未配置密钥,请输入登录密码。
        </div>
        <input
          className="input"
          type="password"
          autoFocus
          value={pw}
          onChange={(e) => setPw(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && pw && onSubmit(pw)}
          placeholder="密码"
          style={{ width: "100%" }}
        />
        {error && (
          <div style={{ fontSize: 12, color: "var(--danger-text)", marginTop: 10, lineHeight: 1.5 }}>{error}</div>
        )}
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 18 }}>
          <Button variant="ghost" onClick={onClose}>取消</Button>
          <Button variant="primary" disabled={!pw} loading={busy} onClick={() => onSubmit(pw)}>
            连接
          </Button>
        </div>
      </div>
    </div>
  );
}
