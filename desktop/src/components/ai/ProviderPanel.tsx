import { useEffect, useState } from "react";
import { Check, Pencil, Plus, Trash2, X } from "lucide-react";
import { Button } from "@/components/ui";
import { ProviderForm } from "@/components/ai/ProviderForm";
import { useAiStore } from "@/stores/ai";
import type { AiProvider } from "@/types";
import { confirmAction } from "@/lib/dialog";

/** 模型列表 + 增删改切换 */
export function ProviderPanel({ onClose }: { onClose: () => void }) {
  const { settings, load, setActive, deleteProvider } = useAiStore();
  const [editing, setEditing] = useState<AiProvider | null>(null);
  const [adding, setAdding] = useState(false);

  useEffect(() => {
    void load();
  }, [load]);

  if (adding || editing) {
    return (
      <div className="vscroll panel-scroll" style={{ padding: 0 }}>
        <div className="panel-head">
          <span className="panel-title">{editing ? "编辑模型" : "添加模型"}</span>
        </div>
        <ProviderForm
          edit={editing ?? undefined}
          onDone={() => { setAdding(false); setEditing(null); }}
          onCancel={() => { setAdding(false); setEditing(null); }}
        />
      </div>
    );
  }

  return (
    <div className="panel">
      <div className="panel-head">
        <span className="panel-title grow">AI 模型</span>
        <div className="icon-action" onClick={onClose}>
          <X size={15} />
        </div>
      </div>
      <div className="panel-hint">
        选中的模型用于侧栏对话与终端 AI 协助。密钥经设备密钥加密存本地,不上传。
      </div>

      <div className="col gap-8 vscroll panel-scroll">
        {settings.providers.length === 0 && (
          <div className="fs-sm t-faint lh-relaxed" style={{ padding: "10px 2px" }}>
            还没有配置模型。点下方「添加模型」,可选 Claude、OpenAI、DeepSeek、任意 OpenAI 兼容服务或本地 Ollama。
          </div>
        )}
        {settings.providers.map((p) => {
          const active = p.id === settings.activeId;
          return (
            <div
              key={p.id}
              className="pick-card"
              data-active={active ? "true" : "false"}
              onClick={() => void setActive(p.id)}
            >
              <div className="grow lh-snug">
                <div className="row gap-6">
                  {active && <Check size={12} className="t-accent fnone" />}
                  <span className="fs-sm w-semi t-strong ellipsis">{p.name}</span>
                  {!p.hasKey && p.protocol !== "ollama" && (
                    <span className="fs-2xs t-warning fnone">缺密钥</span>
                  )}
                </div>
                <div className="mono fs-2xs t-faint ellipsis">
                  {p.model} · {p.protocol}
                </div>
              </div>
              <div
                className="icon-action sm"
                title="编辑"
                onClick={(e) => { e.stopPropagation(); setEditing(p); }}
              >
                <Pencil size={13} />
              </div>
              <div
                className="icon-action sm"
                title="删除"
                onClick={async (e) => {
                  e.stopPropagation();
                  if (await confirmAction(`删除模型「${p.name}」?`, "删除模型")) await deleteProvider(p.id);
                }}
              >
                <Trash2 size={13} />
              </div>
            </div>
          );
        })}
      </div>

      <div className="panel-foot">
        <Button variant="secondary" className="w-full" onClick={() => setAdding(true)}>
          <Plus size={14} />添加模型
        </Button>
      </div>
    </div>
  );
}
