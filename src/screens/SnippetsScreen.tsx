import { useMemo, useState } from "react";
import { Pencil, Trash2 } from "lucide-react";
import { Button, ScreenHeader, TextInput } from "@/components/ui";
import { useDataStore } from "@/stores/data";
import { useSessionStore } from "@/stores/session";
import { useUiStore } from "@/stores/ui";
import type { Snippet } from "@/types";

function SnippetModal({ edit, onClose }: { edit?: Snippet; onClose: () => void }) {
  const saveSnippet = useDataStore((s) => s.saveSnippet);
  const [name, setName] = useState(edit?.name ?? "");
  const [command, setCommand] = useState(edit?.command ?? "");
  const [tags, setTags] = useState(edit?.tags.join(", ") ?? "");
  return (
    <div className="modal-overlay" onClick={onClose}>
      <div className="modal" onClick={(e) => e.stopPropagation()}>
        <h3 className="modal-title">{edit ? "编辑片段" : "新建片段"}</h3>
        <div className="col gap-14">
          <TextInput label="名称" placeholder="重启网关" value={name} onChange={(e) => setName(e.target.value)} />
          <div>
            <label className="input-label">命令(支持 {"{{cursor}}"} 占位符)</label>
            <textarea
              value={command}
              onChange={(e) => setCommand(e.target.value)}
              placeholder="pm2 restart gateway"
              className="textarea"
            />
          </div>
          <TextInput label="标签(逗号分隔)" placeholder="生产, pm2" value={tags} onChange={(e) => setTags(e.target.value)} />
        </div>
        <div className="modal-actions">
          <Button variant="ghost" onClick={onClose}>取消</Button>
          <Button
            variant="primary"
            disabled={!name.trim() || !command.trim()}
            onClick={async () => {
              await saveSnippet({ id: edit?.id, name: name.trim(), command: command.trim(), tags: tags.split(",").map((t) => t.trim()).filter(Boolean) });
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

export function SnippetsScreen() {
  const { snippets, bumpSnippet, deleteSnippet } = useDataStore();
  const runCommand = useSessionStore((s) => s.runCommand);
  const sessions = useSessionStore((s) => s.sessions);
  const go = useUiStore((s) => s.go);
  const [query, setQuery] = useState("");
  const [showNew, setShowNew] = useState(false);
  const [editSnip, setEditSnip] = useState<Snippet | null>(null);
  const [copied, setCopied] = useState<string | null>(null);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return snippets;
    return snippets.filter((s) => (s.name + " " + s.command + " " + s.tags.join(" ")).toLowerCase().includes(q));
  }, [snippets, query]);

  const run = async (id: string, command: string) => {
    await bumpSnippet(id);
    if (sessions.length > 0) {
      await runCommand(command);
      go("terminal");
    } else {
      go("hosts");
    }
  };

  return (
    <div className="screen vscroll">
      <div className="screen-inner wide">
        <ScreenHeader eyebrow="SNIPPETS" title="命令片段">
          <TextInput className="search" placeholder="搜索片段…" value={query} onChange={(e) => setQuery(e.target.value)} />
          <Button variant="primary" onClick={() => setShowNew(true)}>新建片段</Button>
        </ScreenHeader>

        <div className="grid-2">
          {filtered.map((s) => (
            <div key={s.id} className="card hover-card col gap-10 pad-card">
              <div className="row gap-8">
                <span className="grow w-bold fs-md t-strong">{s.name}</span>
                <span className="fs-xs t-faint">用过 {s.useCount} 次</span>
                <div
                  className="icon-action sm"
                  onClick={() => setEditSnip(s)}
                  title="编辑"
                >
                  <Pencil size={13} />
                </div>
                <div
                  className="icon-action sm"
                  onClick={() => { if (confirm(`删除片段「${s.name}」?`)) void deleteSnippet(s.id); }}
                >
                  <Trash2 size={13} />
                </div>
              </div>
              <div
                className="mono fs-sm t-body r-md ellipsis lh-normal bg-sunken bd-subtle p-10"
              >
                {s.command}
              </div>
              <div className="row gap-6">
                {s.tags.map((t) => <span key={t} className="tag-pill">{t}</span>)}
                <span className="spacer" />
                <Button
                  variant="ghost"
                  onClick={async () => {
                    await navigator.clipboard.writeText(s.command);
                    setCopied(s.id);
                    setTimeout(() => setCopied(null), 1500);
                  }}
                >
                  {copied === s.id ? "已复制 ✓" : "复制"}
                </Button>
                <Button variant="secondary" onClick={() => void run(s.id, s.command)}>运行</Button>
              </div>
            </div>
          ))}
        </div>

        {filtered.length === 0 && (
          <div className="card empty-state boxed fs-md">
            没有匹配的片段
          </div>
        )}
      </div>
      {showNew && <SnippetModal onClose={() => setShowNew(false)} />}
      {editSnip && <SnippetModal edit={editSnip} onClose={() => setEditSnip(null)} />}
    </div>
  );
}
