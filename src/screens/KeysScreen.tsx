import { useState } from "react";
import { KeyRound, Lock, ShieldCheck, Trash2 } from "lucide-react";
import { Badge, Button, ScreenHeader, Segmented, TextInput } from "@/components/ui";
import { useDataStore } from "@/stores/data";
import { ipc } from "@/ipc";

function GenerateModal({ onClose }: { onClose: () => void }) {
  const generateKey = useDataStore((s) => s.generateKey);
  const [name, setName] = useState("");
  const [algo, setAlgo] = useState<"ed25519" | "rsa4096">("ed25519");
  const [busy, setBusy] = useState(false);
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3 style={{ fontSize: 17, marginBottom: 16 }}>生成新密钥</h3>
        <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
          <TextInput label="名称" placeholder="id_ed25519_new" value={name} onChange={(e) => setName(e.target.value)} />
          <div>
            <label className="input-label">算法</label>
            <Segmented
              options={[{ value: "ed25519", label: "Ed25519(推荐)" }, { value: "rsa4096", label: "RSA 4096" }]}
              value={algo}
              onChange={setAlgo}
            />
          </div>
        </div>
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 20 }}>
          <Button variant="ghost" onClick={onClose}>取消</Button>
          <Button
            variant="primary"
            disabled={!name.trim()}
            loading={busy}
            onClick={async () => {
              setBusy(true);
              try { await generateKey(name.trim(), algo); onClose(); }
              finally { setBusy(false); }
            }}
          >
            生成
          </Button>
        </div>
      </div>
    </div>
  );
}

function ImportModal({ onClose }: { onClose: () => void }) {
  const importKey = useDataStore((s) => s.importKey);
  const [name, setName] = useState("");
  const [pem, setPem] = useState("");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3 style={{ fontSize: 17, marginBottom: 16 }}>导入私钥</h3>
        <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
          <TextInput label="名称" placeholder="id_ed25519_imported" value={name} onChange={(e) => setName(e.target.value)} />
          <div>
            <label className="input-label">私钥内容(OpenSSH PEM)</label>
            <textarea
              value={pem}
              onChange={(e) => setPem(e.target.value)}
              placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
              style={{ width: "100%", height: 140, resize: "vertical", background: "var(--surface-sunken)", border: "1px solid var(--border-default)", borderRadius: 10, padding: 10, fontFamily: "var(--font-mono)", fontSize: 11.5, color: "var(--text-strong)", outline: "none" }}
            />
          </div>
          {err && <div style={{ fontSize: 12, color: "var(--danger-text)" }}>{err}</div>}
        </div>
        <div style={{ display: "flex", justifyContent: "flex-end", gap: 8, marginTop: 20 }}>
          <Button variant="ghost" onClick={onClose}>取消</Button>
          <Button
            variant="primary"
            disabled={!name.trim() || !pem.trim()}
            loading={busy}
            onClick={async () => {
              setBusy(true);
              setErr(null);
              try { await importKey(name.trim(), pem); onClose(); }
              catch (e) { setErr(String(e)); }
              finally { setBusy(false); }
            }}
          >
            导入
          </Button>
        </div>
      </div>
    </div>
  );
}

export function KeysScreen() {
  const { keys, deleteKey } = useDataStore();
  const [showGen, setShowGen] = useState(false);
  const [showImport, setShowImport] = useState(false);
  const [copied, setCopied] = useState<string | null>(null);

  const copyPub = async (id: string) => {
    const pub = await ipc.getPublicKey(id);
    await navigator.clipboard.writeText(pub);
    setCopied(id);
    setTimeout(() => setCopied(null), 1500);
  };

  return (
    <div className="screen vscroll">
      <div className="screen-inner" style={{ maxWidth: 880 }}>
        <ScreenHeader eyebrow="SSH KEYS" title="密钥管理">
          <Button variant="outline" onClick={() => setShowImport(true)}>导入</Button>
          <Button variant="primary" onClick={() => setShowGen(true)}>生成新密钥</Button>
        </ScreenHeader>

        <div className="card" style={{ display: "flex", alignItems: "center", gap: 12, padding: "14px 18px", marginBottom: 16 }}>
          <div className="icon-chip" style={{ width: 34, height: 34, background: "var(--success-subtle)", color: "var(--success-text)" }}>
            <ShieldCheck size={17} />
          </div>
          <div style={{ flex: 1, lineHeight: 1.3 }}>
            <div style={{ fontWeight: 600, fontSize: 13.5, color: "var(--text-strong)" }}>ssh-agent 运行中</div>
            <div style={{ fontSize: 12, color: "var(--text-faint)" }}>已加载 {keys.length} 把密钥 · 解锁方式:Touch ID</div>
          </div>
          <Badge color="success" dot>正常</Badge>
        </div>

        <div style={{ display: "flex", flexDirection: "column", gap: 12 }}>
          {keys.map((k) => (
            <div key={k.id} className="card hover-card" style={{ padding: "16px 18px", display: "flex", alignItems: "center", gap: 14 }}>
              <div className="icon-chip" style={{ width: 38, height: 38 }}><KeyRound size={17} /></div>
              <div style={{ flex: 1, minWidth: 0, lineHeight: 1.45 }}>
                <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
                  <span style={{ fontFamily: "var(--font-mono)", fontWeight: 600, fontSize: 13.5, color: "var(--text-strong)" }}>{k.name}</span>
                  <Badge color="neutral">{k.algo === "ed25519" ? "ED25519" : "RSA 4096"}</Badge>
                  <Badge color={k.syncEnabled ? "success" : "neutral"} dot>{k.syncEnabled ? "已同步" : "仅本地"}</Badge>
                </div>
                <div style={{ fontFamily: "var(--font-mono)", fontSize: 11, color: "var(--text-faint)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{k.fingerprint}</div>
                <div style={{ fontSize: 11.5, color: "var(--text-muted)" }}>
                  {k.usedByHosts > 0 ? `${k.usedByHosts} 台主机使用` : "未使用"} · {k.createdAt} 创建
                </div>
              </div>
              <Button variant="ghost" onClick={() => void copyPub(k.id)}>
                {copied === k.id ? "已复制 ✓" : "复制公钥"}
              </Button>
              <div
                className="hover-row"
                onClick={() => { if (confirm(`删除密钥 ${k.name}?此操作不可恢复。`)) void deleteKey(k.id); }}
                style={{ padding: 6, borderRadius: 8, cursor: "pointer", color: "var(--text-faint)" }}
              >
                <Trash2 size={16} />
              </div>
            </div>
          ))}
        </div>

        <div style={{ display: "flex", alignItems: "center", gap: 8, marginTop: 14, fontSize: 12, color: "var(--text-faint)" }}>
          <Lock size={13} />
          私钥永不明文离开本机;云同步前使用主密码在本地加密(XChaCha20-Poly1305)。
        </div>
      </div>
      {showGen && <GenerateModal onClose={() => setShowGen(false)} />}
      {showImport && <ImportModal onClose={() => setShowImport(false)} />}
    </div>
  );
}
