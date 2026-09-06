# KiteTerm 管理平台 —— 生产部署

## 二进制 + systemd(推荐)

```bash
tar xzf kite-server-<版本>-linux-<架构>.tar.gz && cd kite-server-*
sudo install -m 755 kite-server /usr/local/bin/kite-server
sudo install -d -m 700 /etc/kite-server
sudo install -m 600 kite-server.env.example /etc/kite-server/kite-server.env
sudo vi /etc/kite-server/kite-server.env        # 至少填 KITE_ADMIN_KEY 与 S3_*
sudo install -m 644 kite-server.service /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now kite-server
sudo journalctl -u kite-server -f                # 看到 "listening on" 即就绪
```

- 控制台:`http://<host>:8787/admin`,输入 `KITE_ADMIN_KEY`。首次进入到「S3 存储」页点「测试连接」确认桶可读写。
- 数据只有一个 SQLite 文件(`/var/lib/kite-server/kite-server.db`),备份它即可;S3 secret 与 TOTP 密钥明文存于其中,保持目录 0700。
- 升级:替换 `/usr/local/bin/kite-server` 后 `systemctl restart kite-server`,库表迁移在启动时自动完成、幂等。
- 生产建议 `KITE_STS=require`:`auto` 在 STS 故障时会回退下发共享根凭据。
- 对外请用 Nginx/Caddy 做 TLS 反代并把 `KITE_LISTEN` 留在 127.0.0.1;反代需透传 `X-Forwarded-For`(注册限流按客户端 IP)。

## Docker

```bash
docker build -t kite-server:<版本> server/
docker run -d --name kite-server --restart unless-stopped \
  --env-file kite-server.env -e KITE_LISTEN=0.0.0.0:8787 -e KITE_DB=/data/kite-server.db \
  -p 127.0.0.1:8787:8787 -v kite-data:/data kite-server:<版本>
```

## 自己交叉编译

```bash
brew install zig && cargo install cargo-zigbuild
rustup target add x86_64-unknown-linux-musl aarch64-unknown-linux-musl
server/deploy/build-release.sh          # 产物在 server/dist/
```
