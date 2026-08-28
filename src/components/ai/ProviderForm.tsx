import { useState } from "react";
import { Button, TextInput } from "@/components/ui";
import { useAiStore } from "@/stores/ai";
import type { AiProvider } from "@/types";

/** 常见服务的接入预设 —— 端点与协议是固定的,模型 id 用户可改 */
export const PRESETS: { label: string; protocol: AiProvider["protocol"]; baseUrl: string; model: string }[] = [
  { label: "Anthropic Claude", protocol: "anthropic", baseUrl: "https://api.anthropic.com", model: "claude-opus-5" },
  { label: "OpenAI", protocol: "openai", baseUrl: "https://api.openai.com", model: "gpt-4o" },
  { label: "DeepSeek", protocol: "openai", baseUrl: "https://api.deepseek.com", model: "deepseek-chat" },
  { label: "OpenAI 兼容(自填)", protocol: "openai", baseUrl: "", model: "" },
  { label: "Ollama 本地", protocol: "ollama", baseUrl: "http://127.0.0.1:11434", model: "llama3.1" },
];

export const PROTOCOL_LABEL: Record<AiProvider["protocol"], string> = {
  anthropic: "Anthropic /v1/messages",
  openai: "OpenAI 兼容 /v1/chat/completions",
  ollama: "Ollama /api/chat",
};

/** 模型编辑表单 */
export function ProviderForm({
  edit,
  onDone,
  onCancel,
}: {
  edit?: AiProvider;
  onDone: () => void;
  onCancel: () => void;
}) {
  const save = useAiStore((s) => s.saveProvider);
  const [form, setForm] = useState<Omit<AiProvider, "hasKey">>(
    edit
      ? { id: edit.id, name: edit.name, protocol: edit.protocol, baseUrl: edit.baseUrl, model: edit.model }
      : { id: "", name: "", protocol: "anthropic", baseUrl: PRESETS[0].baseUrl, model: PRESETS[0].model },
  );
  const [apiKey, setApiKey] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const set = <K extends keyof typeof form>(k: K, v: (typeof form)[K]) =>
    setForm((f) => ({ ...f, [k]: v }));

  const applyPreset = (label: string) => {
    const p = PRESETS.find((x) => x.label === label);
    if (!p) return;
    setForm((f) => ({
      ...f,
      protocol: p.protocol,
      baseUrl: p.baseUrl,
      model: p.model,
      name: f.name || (p.baseUrl ? p.label : f.name),
    }));
  };

  const needKey = form.protocol !== "ollama";

  return (
    <div className="col gap-10 panel-body">
      {!edit && (
        <div>
          <label className="input-label">快速选择</label>
          <select
            className="input w-full"
            defaultValue={PRESETS[0].label}
            onChange={(e) => applyPreset(e.target.value)}
          >
            {PRESETS.map((p) => (
              <option key={p.label} value={p.label}>{p.label}</option>
            ))}
          </select>
        </div>
      )}
      <TextInput
        label="名称"
        placeholder="给这个模型起个名字"
        value={form.name}
        onChange={(e) => set("name", e.target.value)}
      />
      <div>
        <label className="input-label">接口协议</label>
        <select
          className="input w-full"
          value={form.protocol}
          onChange={(e) => set("protocol", e.target.value as AiProvider["protocol"])}
        >
          {(Object.keys(PROTOCOL_LABEL) as AiProvider["protocol"][]).map((k) => (
            <option key={k} value={k}>{PROTOCOL_LABEL[k]}</option>
          ))}
        </select>
      </div>
      <TextInput
        label="接口地址"
        placeholder="https://api.example.com"
        value={form.baseUrl}
        onChange={(e) => set("baseUrl", e.target.value)}
      />
      <TextInput
        label="模型 id"
        placeholder="claude-opus-5 / gpt-4o / llama3.1"
        value={form.model}
        onChange={(e) => set("model", e.target.value)}
      />
      {needKey && (
        <TextInput
          label={edit?.hasKey ? "API Key(留空则保持不变)" : "API Key"}
          type="password"
          placeholder={edit?.hasKey ? "••••••••" : "sk-…"}
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
        />
      )}
      {error && <div className="fs-sm t-danger lh-normal">{error}</div>}
      <div className="row jend gap-8">
        <Button variant="ghost" onClick={onCancel}>取消</Button>
        <Button
          variant="primary"
          loading={busy}
          disabled={!form.name.trim() || !form.baseUrl.trim() || !form.model.trim()}
          onClick={async () => {
            setBusy(true);
            setError(null);
            try {
              await save(form, apiKey.trim() === "" ? undefined : apiKey.trim());
              onDone();
            } catch (e) {
              setError(String(e).replace(/^[A-Za-z_]+:\s*/, ""));
            } finally {
              setBusy(false);
            }
          }}
        >
          保存
        </Button>
      </div>
    </div>
  );
}
