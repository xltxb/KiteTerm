import { useState } from "react";
import { Button } from "@/components/ui";
import { ipc } from "@/ipc";

/** 输入旧设备抄写的 24 词短语,在本机重建主密钥 */
export function RestoreModal({ onClose, onDone }: { onClose: () => void; onDone: () => void }) {
  const [phrase, setPhrase] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal lg" onClick={(e) => e.stopPropagation()}>
        <h3 className="modal-title tight">用恢复短语恢复主密钥</h3>
        <div className="fs-sm t-muted lh-relaxed mb-12">
          输入在旧设备上抄写的 24 个单词(空格分隔),即可在本机重建主密钥并解密同步数据。
        </div>
        <textarea
          value={phrase}
          onChange={(e) => setPhrase(e.target.value)}
          placeholder="word1 word2 word3 …"
          className="textarea"
        />
        {error && <div className="fs-sm t-danger mt-8">{error}</div>}
        <div className="row jend gap-8 mt-16">
          <Button variant="ghost" onClick={onClose}>取消</Button>
          <Button
            variant="primary"
            loading={busy}
            disabled={phrase.trim().split(/\s+/).length !== 24}
            onClick={async () => {
              setBusy(true);
              setError(null);
              try {
                await ipc.syncRestoreMaster(phrase);
                onDone();
              } catch (e) {
                setError(String(e));
              } finally {
                setBusy(false);
              }
            }}
          >
            恢复
          </Button>
        </div>
      </div>
    </div>
  );
}
