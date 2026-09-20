// 证书(SSL / ACME)界面
import { useEffect, useState } from "react";
import { message as tauriMessage } from "@tauri-apps/plugin-dialog";
import {
  Check,
  Circle,
  Copy,
  Download,
  Ellipsis,
  FileText,
  Loader2,
  Lock,
  PauseCircle,
  PlugZap,
  RefreshCw,
  Server,
  Trash2,
  X,
} from "lucide-react";
import { Badge, Button, Progress, ScreenHeader, Segmented, TextInput } from "@/components/ui";
import { confirmAction, showError } from "@/lib/dialog";
import { useCertsStore } from "@/stores/certs";
import { useDataStore } from "@/stores/data";
import { useSessionStore } from "@/stores/session";
import { useSyncStore } from "@/stores/sync";
import { useUiStore } from "@/stores/ui";
import type { CertInfo } from "@/types";

type ChallengeKind = CertInfo["challenge"];

/** 签发五步流水线的默认文案(某一步事件尚未到达时用它占位) */
const STEP_LABELS = ["校验域名", "完成挑战", "签发证书", "上传部署", "重载服务"];

function caLabel(ca: CertInfo["ca"]): string {
  return ca === "letsencrypt" ? "Let's Encrypt" : "ZeroSSL";
}

function challengeLabel(c: ChallengeKind): string {
  return c === "http" ? "HTTP-01" : "DNS-01";
}

const STATE_BADGE: Record<CertInfo["state"], { color: "success" | "warning" | "danger"; text: string }> = {
  active: { color: "success", text: "有效" },
  expiring: { color: "warning", text: "即将到期" },
  error: { color: "danger", text: "错误" },
};

/** 有效期进度条颜色:剩余天数越少越警示 */
function barColor(daysLeft: number): string {
  if (daysLeft < 7) return "var(--danger)";
  if (daysLeft <= 30) return "var(--warning)";
  return "var(--accent)";
}

type MenuState = { certId: string; mode: "main" | "deploy" };

export function CertsScreen() {
  const { certs, issuing, issueSteps, dnsRecord, load, issue, renew, revoke, deploy, confirmDns, downloadChain } =
    useCertsStore();
  const hosts = useDataStore((s) => s.hosts);
  const sessions = useSessionStore((s) => s.sessions);
  const go = useUiStore((s) => s.go);
  const devices = useSyncStore((s) => s.status?.devices ?? []);
  const certsSynced = useSyncStore((s) => s.config?.scopes.certs ?? false);
  const myId = devices.find((d) => d.current)?.id ?? "";
  const ownerLabel = (c: CertInfo) => {
    // devices 来自桶内 manifest;未配置同步时为空,此时本机就是唯一设备
    if (myId === "" || !c.ownerDeviceId || c.ownerDeviceId === myId) return "本机自动续期";
    const d = devices.find((x) => x.id === c.ownerDeviceId);
    return `由 ${d?.name ?? c.ownerDeviceId.slice(0, 8)} 自动续期`;
  };
  const isTakeover = (c: CertInfo) => myId !== "" && !!c.ownerDeviceId && c.ownerDeviceId !== myId;

  const [domains, setDomains] = useState("");
  const [challenge, setChallenge] = useState<ChallengeKind>("http");
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [toast, setToast] = useState<string | null>(null);

  useEffect(() => { void load(); }, [load]);

  const showToast = (text: string) => {
    setToast(text);
    setTimeout(() => setToast((t) => (t === text ? null : t)), 1800);
  };

  const hostName = (id: string) => hosts.find((h) => h.id === id)?.name ?? id;
  const activeSession = sessions.find((s) => s.kind === "ssh" && s.state === "connected");
  const activeCount = certs.filter((c) => c.state === "active").length;

  const domainList = domains.split(/[,\s]+/).map((d) => d.trim()).filter(Boolean);

  const handleIssue = async () => {
    if (!activeSession) return;
    if (domainList.length === 0) {
      await showError("请至少输入一个域名");
      return;
    }
    await issue({
      domains: domainList,
      challenge,
      ca: "letsencrypt",
      hostId: activeSession.hostId,
      deployPath: "/etc/nginx/ssl/fullchain.pem",
      reloadCmd: "sudo nginx -s reload",
    });
  };

  const showDetails = async (c: CertInfo) => {
    await tauriMessage(
      [
        `域名:${c.domains.join(", ")}`,
        `CA:${caLabel(c.ca)}`,
        `验证方式:${challengeLabel(c.challenge)}`,
        `部署主机:${hostName(c.hostId)}`,
        `部署路径:${c.deployPath}`,
        `重载命令:${c.reloadCmd}`,
        `到期时间:${new Date(c.notAfter).toLocaleString()}`,
        `自动续期:${c.autoRenew ? "已开启" : "已关闭"}`,
      ].join("\n"),
      { title: "证书详情", kind: "info" },
    );
  };

  const copyPath = async (c: CertInfo) => {
    await navigator.clipboard.writeText(c.deployPath);
    showToast("证书路径已复制到剪贴板");
  };

  const revokeCert = async (c: CertInfo) => {
    if (await confirmAction(`吊销并删除证书 ${c.domains.join(", ")}?此操作不可恢复。`, "吊销证书")) {
      await revoke(c.id);
      showToast("证书已吊销");
    }
  };

  return (
    <div className="screen vscroll" onClick={() => setMenu(null)}>
      <div className="screen-inner medium" style={{ position: "relative" }}>
        <ScreenHeader eyebrow="SSL CERTIFICATES" title="SSL 证书" count={`${activeCount} 有效`} />

        {/* 签发表单 */}
        <div className="card pad-card mb-16" style={{ opacity: activeSession ? 1 : 0.6 }}>
          <div className="row gap-8 mb-12">
            <span className="fs-sm w-bold t-muted">一键申请 · ACME (Let's Encrypt / ZeroSSL)</span>
            <span className="spacer" />
            {activeSession && (
              <Badge color="success" dot>{activeSession.hostName} 已连接</Badge>
            )}
          </div>

          {activeSession ? (
            <>
              <div className="row wrap aend gap-10">
                <TextInput
                  label="域名"
                  placeholder="example.com, *.example.com"
                  value={domains}
                  onChange={(e) => setDomains(e.target.value)}
                  style={{ width: 240 }}
                />
                <TextInput
                  label="部署主机(仅限已连接)"
                  value={activeSession.hostName}
                  readOnly
                  style={{ width: 180 }}
                />
                <Segmented
                  mono
                  options={[{ value: "http", label: "HTTP-01" }, { value: "dns", label: "DNS-01" }]}
                  value={challenge}
                  onChange={setChallenge}
                />
                <Button variant="primary" loading={issuing} disabled={domainList.length === 0} onClick={handleIssue}>
                  申请并部署
                </Button>
              </div>
              <div className="fs-xs t-faint mt-8">
                HTTP-01 经当前 SSH 会话在主机放置校验文件;DNS-01 支持通配符,需配置 DNS API。签发后自动上传证书、重载 nginx,并加入自动续期。
              </div>
            </>
          ) : (
            <div className="row gap-12">
              <div className="icon-chip md" data-tone="neutral">
                <PlugZap size={17} />
              </div>
              <div className="grow lh-normal">
                <div className="fs-md w-semi t-strong">未连接任何主机</div>
                <div className="fs-xs t-faint">证书的申请与部署经 SSH 会话执行,请先连接目标主机。</div>
              </div>
              <Button variant="secondary" onClick={() => go("hosts")}>去连接主机</Button>
            </div>
          )}
        </div>

        {/* 签发中:五步流水线 */}
        {issuing && (
          <div className="card pad-card mb-16" style={{ borderColor: "var(--accent-subtle-border)" }}>
            <div className="col gap-8">
              {STEP_LABELS.map((defaultLabel, i) => {
                const step = issueSteps[i];
                const state: "pending" | "running" | "ok" | "error" = step?.state ?? "pending";
                const label = step?.label ?? defaultLabel;
                let tone = "t-faint";
                let icon = <Circle size={14} />;
                if (state === "ok") { tone = "t-success"; icon = <Check size={14} />; }
                else if (state === "error") { tone = "t-danger"; icon = <X size={14} />; }
                else if (state === "running") { tone = "t-accent"; icon = <Loader2 size={14} className="spin" />; }
                return (
                  <div key={i} className={`row gap-10 fs-sm ${tone}`}>
                    {icon}
                    <span className="mono">{label}</span>
                  </div>
                );
              })}
            </div>
          </div>
        )}

        {/* DNS-01 手动验证 */}
        {dnsRecord && (
          <div className="card pad-card mb-16">
            <div className="fs-sm w-bold t-muted mb-12">DNS-01 手动验证 · 请在域名解析中添加以下 TXT 记录</div>
            <div className="col gap-8">
              <div className="row gap-8">
                <span className="fs-xs t-faint" style={{ width: 40 }}>名称</span>
                <span className="mono fs-sm t-strong grow ellipsis">{dnsRecord.name}</span>
                <div
                  className="icon-btn sm plain"
                  onClick={() => { void navigator.clipboard.writeText(dnsRecord.name); showToast("记录名已复制"); }}
                >
                  <Copy size={14} />
                </div>
              </div>
              <div className="row gap-8">
                <span className="fs-xs t-faint" style={{ width: 40 }}>值</span>
                <span className="mono fs-sm t-strong grow ellipsis">{dnsRecord.value}</span>
                <div
                  className="icon-btn sm plain"
                  onClick={() => { void navigator.clipboard.writeText(dnsRecord.value); showToast("记录值已复制"); }}
                >
                  <Copy size={14} />
                </div>
              </div>
            </div>
            <div className="row jend mt-12">
              <Button variant="primary" onClick={() => void confirmDns(dnsRecord.certId)}>我已添加,验证</Button>
            </div>
          </div>
        )}

        {/* 证书列表 */}
        <div className="card">
          {certs.length === 0 && (
            <div className="empty-state boxed fs-md">还没有已签发的证书</div>
          )}
          {certs.map((c) => {
            const daysLeft = Math.floor((c.notAfter - Date.now()) / 86400000);
            const pct = Math.max(0, Math.min(100, (daysLeft / 90) * 100));
            const badge = STATE_BADGE[c.state];
            const menuOpen = menu?.certId === c.id;
            return (
              <div key={c.id} className="list-row">
                <div className="icon-chip md" data-tone={badge.color}>
                  <Lock size={16} />
                </div>
                <div className="grow lh-normal min-w-0">
                  <div className="row gap-8">
                    <span className="mono w-semi fs-md t-strong ellipsis">{c.domains.join(", ")}</span>
                    <Badge color={badge.color} dot>{badge.text}</Badge>
                  </div>
                  <div className="fs-xs t-faint">{caLabel(c.ca)} · {challengeLabel(c.challenge)} · {ownerLabel(c)}</div>
                </div>
                <div style={{ width: 150, flex: "none" }}>
                  <div className="row between fs-2xs t-faint mb-4">
                    <span>{daysLeft} 天</span><span>90 天</span>
                  </div>
                  <Progress value={pct} className="thin" style={{ color: barColor(daysLeft) }} />
                </div>
                <span className="row gap-6 fs-xs t-muted" style={{ width: 130 }}>
                  <Server size={12} />{hostName(c.hostId)}
                </span>
                <Button variant="ghost" size="sm" onClick={() => void renew(c.id)}>
                  {isTakeover(c) ? "接管并续期" : "立即续期"}
                </Button>
                <div style={{ position: "relative" }} onClick={(e) => e.stopPropagation()}>
                  <div
                    className="icon-btn sm plain"
                    onClick={() => setMenu(menuOpen ? null : { certId: c.id, mode: "main" })}
                  >
                    <Ellipsis size={16} />
                  </div>
                  {menuOpen && menu?.mode === "main" && (
                    <div className="cert-menu">
                      <div className="cert-menu-item" onClick={() => { setMenu(null); void showDetails(c); }}>
                        <FileText size={14} /><span className="grow">查看证书详情</span>
                      </div>
                      <div className="cert-menu-item" onClick={() => { setMenu(null); void copyPath(c); }}>
                        <Copy size={14} /><span className="grow">复制证书路径</span>
                      </div>
                      <div className="cert-menu-item" onClick={() => { setMenu(null); void downloadChain(c.id); }}>
                        <Download size={14} /><span className="grow">下载证书链</span>
                      </div>
                      <div className="cert-menu-item" onClick={() => setMenu({ certId: c.id, mode: "deploy" })}>
                        <Server size={14} /><span className="grow">部署到其他主机…</span>
                      </div>
                      <div className="cert-menu-item" onClick={() => { setMenu(null); void renew(c.id); }}>
                        <RefreshCw size={14} /><span className="grow">重试验证</span>
                      </div>
                      <div
                        className="cert-menu-item"
                        onClick={() => { setMenu(null); showToast("本版暂不支持暂停续期"); }}
                      >
                        <PauseCircle size={14} /><span className="grow">暂停续期</span>
                      </div>
                      <div className="cert-menu-divider" />
                      <div className="cert-menu-item danger" onClick={() => { setMenu(null); void revokeCert(c); }}>
                        <Trash2 size={14} /><span className="grow">吊销删除</span>
                      </div>
                    </div>
                  )}
                  {menuOpen && menu?.mode === "deploy" && (
                    <div className="cert-menu">
                      <div className="fs-2xs t-faint" style={{ padding: "4px 10px 6px" }}>选择部署主机</div>
                      {hosts.map((h) => (
                        <div
                          key={h.id}
                          className="cert-menu-item"
                          onClick={() => { setMenu(null); void deploy(c.id, h.id).then(() => showToast(`已部署到 ${h.name}`)); }}
                        >
                          <Server size={14} /><span className="grow">{h.name}</span>
                        </div>
                      ))}
                      <div className="cert-menu-divider" />
                      <div className="cert-menu-item" onClick={() => setMenu({ certId: c.id, mode: "main" })}>
                        <span className="grow">‹ 返回</span>
                      </div>
                    </div>
                  )}
                </div>
              </div>
            );
          })}
        </div>

        <div className="row gap-8 fs-sm t-faint mt-12">
          <RefreshCw size={13} />
          到期前 30 天自动续期并重载服务;
          {certsSynced ? "账户密钥与证书随主机配置端到端加密同步。" : "证书与私钥仅保存在本机。"}
        </div>
      </div>

      {toast && (
        <div className="cert-toast">
          <Check size={14} />{toast}
        </div>
      )}
    </div>
  );
}
