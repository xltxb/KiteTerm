// 浏览器开发模式下的演示数据 — 内容与原型《Vela Term.dc.html》一致
import type {
  ForwardRule,
  Host,
  HostGroup,
  MonitorSample,
  Recording,
  Snippet,
  SshKey,
  SyncDevice,
} from "@/types";

export const mockGroups: HostGroup[] = [
  { id: "g-prod", name: "生产 PRODUCTION", sort: 0 },
  { id: "g-data", name: "数据 DATA", sort: 1 },
  { id: "g-personal", name: "个人 PERSONAL", sort: 2 },
];

export const mockHosts: Host[] = [
  { id: "h1", name: "prod-web-01", addr: "10.24.0.31", port: 22, user: "lin", groupId: "g-prod", tags: ["nginx", "node"], keyId: "k1", lastConnectedAt: "刚刚 · 已连接", latencyMs: 12 },
  { id: "h2", name: "prod-web-02", addr: "10.24.0.32", port: 22, user: "lin", groupId: "g-prod", tags: ["nginx"], keyId: "k1", lastConnectedAt: "2 小时前", latencyMs: 14 },
  { id: "h3", name: "db-primary", addr: "10.24.1.10", port: 522, user: "admin", groupId: "g-data", tags: ["postgres 16"], keyId: "k1", lastConnectedAt: "昨天", latencyMs: 18 },
  { id: "h4", name: "redis-cache", addr: "10.24.1.12", port: 22, user: "admin", groupId: "g-data", tags: ["redis 7"], keyId: "k1", lastConnectedAt: "3 天前", latencyMs: 26 },
  { id: "h5", name: "pi-home", addr: "192.168.1.8", port: 22, user: "pi", groupId: "g-personal", tags: ["raspbian"], keyId: "k2", lastConnectedAt: "上周", latencyMs: 8 },
  { id: "h6", name: "vps-hk", addr: "45.77.20.6", port: 2222, user: "root", groupId: "g-personal", tags: ["debian 12"], keyId: "k2", lastConnectedAt: "8 月 12 日", latencyMs: 86 },
];

export const mockKeys: SshKey[] = [
  { id: "k1", name: "id_ed25519_work", algo: "ed25519", fingerprint: "SHA256:tQ8mZ3kL9xWv2nR5pY7cA1bD4eF6gH8jK0mN2pQ4rS6", publicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDemo1 lin@vela", syncEnabled: true, createdAt: "2024-11-03", usedByHosts: 4 },
  { id: "k2", name: "id_rsa_personal", algo: "rsa4096", fingerprint: "SHA256:aB3cD5eF7gH9iJ1kL3mN5oP7qR9sT1uV3wX5yZ7aB9c", publicKey: "ssh-rsa AAAAB3NzaDemo2 lin@vela", syncEnabled: true, createdAt: "2023-06-17", usedByHosts: 2 },
  { id: "k3", name: "id_ed25519_ci", algo: "ed25519", fingerprint: "SHA256:xY2zA4bC6dE8fG0hI2jK4lM6nO8pQ0rS2tU4vW6xY8z", publicKey: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIDemo3 lin@vela", syncEnabled: false, createdAt: "2026-08-01", usedByHosts: 0 },
];

export const mockSnippets: Snippet[] = [
  { id: "s1", name: "重启网关", command: "pm2 restart gateway && pm2 logs gateway --lines 20", tags: ["生产", "pm2"], useCount: 32, updatedAt: "2026-08-20" },
  { id: "s2", name: "查看实时日志", command: "tail -f /var/www/vela/logs/realtime.log", tags: ["日志"], useCount: 58, updatedAt: "2026-08-24" },
  { id: "s3", name: "数据库备份", command: "pg_dump -Fc vela > ~/backup/vela_$(date +%F).dump", tags: ["postgres", "备份"], useCount: 11, updatedAt: "2026-08-11" },
  { id: "s4", name: "清理 Docker", command: "docker system prune -af --volumes", tags: ["docker"], useCount: 7, updatedAt: "2026-07-29" },
  { id: "s5", name: "证书续期", command: "certbot renew --nginx --quiet", tags: ["nginx", "tls"], useCount: 4, updatedAt: "2026-07-12" },
  { id: "s6", name: "端口占用", command: "lsof -i :{{cursor}} | head", tags: ["排查"], useCount: 21, updatedAt: "2026-08-25" },
];

export const mockForwards: ForwardRule[] = [
  { id: "f1", type: "L", localPort: 5432, target: "db-primary:5432", viaHostId: "h1", enabled: true, bytesUp: 1_258_291, bytesDown: 50_960_793 },
  { id: "f2", type: "L", localPort: 6379, target: "redis-cache:6379", viaHostId: "h1", enabled: false, bytesUp: 0, bytesDown: 0 },
  { id: "f3", type: "D", localPort: 1080, target: "", viaHostId: "h6", enabled: true, bytesUp: 90_177_536, bytesDown: 1_503_238_553 },
];

export const mockRecordings: Recording[] = [
  { id: "r1", sessionHost: "prod-web-01", name: "部署 v2.4 发布", durationMs: 754_000, startedAt: "今天 09:14", syncEnabled: true },
  { id: "r2", sessionHost: "db-primary", name: "数据库迁移", durationMs: 1_568_000, startedAt: "昨天", syncEnabled: true },
  { id: "r3", sessionHost: "prod-web-02", name: "排查 502", durationMs: 492_000, startedAt: "8 月 20 日", syncEnabled: false },
];

export const mockMonitor: MonitorSample[] = [
  { hostId: "h1", hostName: "prod-web-01", online: true, uptime: "up 42d", cpu: 34, mem: 62, disk: 48, load: "load 0.84", netUp: "↑ 2.1 MB/s", netDown: "↓ 8.4 MB/s" },
  { hostId: "h2", hostName: "prod-web-02", online: true, uptime: "up 42d", cpu: 18, mem: 41, disk: 46, load: "load 0.31", netUp: "↑ 0.8 MB/s", netDown: "↓ 3.2 MB/s" },
  { hostId: "h3", hostName: "db-primary", online: true, uptime: "up 128d", cpu: 57, mem: 78, disk: 66, load: "load 1.62", netUp: "↑ 4.6 MB/s", netDown: "↓ 1.1 MB/s" },
  { hostId: "h6", hostName: "vps-hk", online: true, uptime: "up 9d", cpu: 8, mem: 35, disk: 91, load: "load 0.12", netUp: "↑ 0.1 MB/s", netDown: "↓ 0.4 MB/s" },
];

export const mockDevices: SyncDevice[] = [
  { id: "d1", name: 'MacBook Pro 16"', kind: "laptop", lastSeenAt: "刚刚", current: true },
  { id: "d2", name: "Mac mini · 工作室", kind: "desktop", lastSeenAt: "12 分钟前", current: false },
  { id: "d3", name: "iPhone 15 Pro", kind: "phone", lastSeenAt: "昨天 22:41", current: false },
];

export const mockLocalFiles = [
  { name: "dist", isDir: true, size: 0, modified: "今天 09:02" },
  { name: "assets", isDir: true, size: 0, modified: "昨天" },
  { name: "deploy.sh", isDir: false, size: 2_150, modified: "今天 08:47" },
  { name: "release-v2.4.tar.gz", isDir: false, size: 50_540_749, modified: "今天 09:05" },
  { name: ".env.production", isDir: false, size: 1_229, modified: "8 月 22 日" },
];

export const mockRemoteFiles = [
  { name: "releases", isDir: true, size: 0, modified: "今天 09:14" },
  { name: "shared", isDir: true, size: 0, modified: "8 月 18 日" },
  { name: "ecosystem.config.js", isDir: false, size: 3_481, modified: "8 月 10 日" },
  { name: "nginx.conf", isDir: false, size: 6_963, modified: "7 月 30 日" },
  { name: "realtime.log", isDir: false, size: 134_217_728, modified: "刚刚" },
];
