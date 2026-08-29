import { useState } from "react";
import { KeyRound, Lock, ShieldCheck, Trash2 } from "lucide-react";
import { Badge, Button, ScreenHeader, Segmented, TextInput } from "@/components/ui";
import { useDataStore } from "@/stores/data";
import { ipc } from "@/ipc";
import { confirmAction } from "@/lib/dialog";

function GenerateModal({ onClose }: { onClose: () => void }) {
  const generateKey = useDataStore((s) => s.generateKey);
  const [name, setName] = useState("");
  const [algo, setAlgo] = useState<"ed25519" | "rsa4096">("ed25519");
  const [busy, setBusy] = useState(false);
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3 className="modal-title">生成新密钥</h3>
        <div className="col gap-14">
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
        <div className="modal-actions">
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
        <h3 className="modal-title">导入私钥</h3>
        <div className="col gap-14">
          <TextInput label="名称" placeholder="id_ed25519_imported" value={name} onChange={(e) => setName(e.target.value)} />
          <div>
            <label className="input-label">私钥内容(OpenSSH PEM)</label>
            <textarea
              value={pem}
              onChange={(e) => setPem(e.target.value)}
              placeholder="-----BEGIN OPENSSH PRIVATE KEY-----"
              className="textarea fs-xs"
              style={{ height: 140 }}
            />
          </div>
          {err && <div className="fs-sm t-danger">{err}</div>}
        </div>
        <div className="modal-actions">
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
      <div className="screen-inner medium">
        <ScreenHeader eyebrow="SSH KEYS" title="密钥管理">
          <Button variant="outline" onClick={() => setShowImport(true)}>导入</Button>
          <Button variant="primary" onClick={() => setShowGen(true)}>生成新密钥</Button>
        </ScreenHeader>

        <div className="card row gap-12 pad-card mb-16">
          <div className="icon-chip md" data-tone="success">
            <ShieldCheck size={17} />
          </div>
          <div className="grow lh-snug">
            <div className="fs-md w-semi t-strong">ssh-agent 运行中</div>
            <div className="fs-sm t-faint">已加载 {keys.length} 把密钥 · 解锁方式:Touch ID</div>
          </div>
          <Badge color="success" dot>正常</Badge>
        </div>

        <div className="col gap-12">
          {keys.map((k) => (
            <div key={k.id} className="card hover-card row gap-14 pad-card">
              <div className="icon-chip md"><KeyRound size={17} /></div>
              <div className="grow lh-normal">
                <div className="row gap-8">
                  <span className="mono w-semi fs-md t-strong">{k.name}</span>
                  <Badge color="neutral">{k.algo === "ed25519" ? "ED25519" : "RSA 4096"}</Badge>
                  <Badge color={k.syncEnabled ? "success" : "neutral"} dot>{k.syncEnabled ? "已同步" : "仅本地"}</Badge>
                </div>
                <div className="mono fs-xs t-faint ellipsis">{k.fingerprint}</div>
                <div className="fs-xs t-muted">
                  {k.usedByHosts > 0 ? `${k.usedByHosts} 台主机使用` : "未使用"} · {k.createdAt} 创建
                </div>
              </div>
              <Button variant="ghost" onClick={() => void copyPub(k.id)}>
                {copied === k.id ? "已复制 ✓" : "复制公钥"}
              </Button>
              <div
                className="icon-btn sm plain"
                onClick={async () => { if (await confirmAction(`删除密钥 ${k.name}?此操作不可恢复。`, "删除密钥")) await deleteKey(k.id); }}
              >
                <Trash2 size={16} />
              </div>
            </div>
          ))}
        </div>

        <div className="row gap-8 fs-sm t-faint mt-12">
          <Lock size={13} />
          私钥永不明文离开本机;云同步前使用主密码在本地加密(XChaCha20-Poly1305)。
        </div>
      </div>
      {showGen && <GenerateModal onClose={() => setShowGen(false)} />}
      {showImport && <ImportModal onClose={() => setShowImport(false)} />}
    </div>
  );
}
