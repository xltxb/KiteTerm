import { useEffect, useState } from "react";
import { Button } from "@/components/ui";
import { ipc } from "@/ipc";

/** 展示 24 词恢复短语,供用户抄写留存 */
export function PhraseModal({ onClose }: { onClose: () => void }) {
  const [phrase, setPhrase] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    ipc.syncRecoveryPhrase().then(setPhrase).catch((e) => setError(String(e)));
  }, []);
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal lg" onClick={(e) => e.stopPropagation()}>
        <h3 className="modal-title tight">恢复短语</h3>
        <div className="fs-sm t-warning lh-relaxed mb-12">
          这 24 个单词等价于你的主密钥:请抄写到纸上妥善保管,任何人拿到即可解密你的同步数据。
          忘记主密码且无此短语 = 数据不可恢复。
        </div>
        {error && <div className="fs-sm t-danger">{error}</div>}
        {phrase && (
          <div className="grid-4">
            {phrase.split(" ").map((w, i) => (
              <div key={i} className="row gap-6 r-inner bg-sunken bd-subtle p-8">
                <span className="mono fs-2xs t-faint word-index">{i + 1}</span>
                <span className="mono fs-sm t-strong">{w}</span>
              </div>
            ))}
          </div>
        )}
        <div className="row jend mt-16">
          <Button variant="primary" onClick={onClose}>我已妥善保存</Button>
        </div>
      </div>
    </div>
  );
}
